#!/usr/bin/env bash
# Bring the observability stack up to date, from the compose project
# directory: run by CD on the VM (/opt/chakramcp) after the config has been
# synced into ./observability/, and by self-hosters from a checkout's infra/.
#
#   observability/scripts/deploy.sh "<changed config dirs>"
#
# The argument lists the config directories whose content changed (CD
# hashes them before and after syncing): alloy, prometheus, loki,
# grafana/provisioning, grafana/dashboards, grafana/channels. Changes are
# applied by reload wherever the service supports it, so a config deploy
# never shows up as a restart (the crash-loop alert counts restarts).
# Compose itself recreates a service whose definition or image changed.
#
# The base compose file is docker-compose.yml when present (the VM), else
# docker-compose.prod.yml (a checkout); COMPOSE_BASE overrides it.
set -euo pipefail

changed=" ${1:-} "
if [ -z "${COMPOSE_BASE:-}" ]; then
  if [ -f docker-compose.yml ]; then COMPOSE_BASE=docker-compose.yml; else COMPOSE_BASE=docker-compose.prod.yml; fi
fi
obs() { docker compose -f "$COMPOSE_BASE" -f observability/compose.yml "$@"; }
# HTTP clients: Loki's and Alloy's images have none, and the static busybox
# wget in Prometheus's image can't resolve Docker DNS names, so Prometheus
# is called on its own localhost and everything else through Grafana's curl.
in_prom() { obs exec -T prometheus wget -qO- "$@"; }
http() { obs exec -T grafana curl -fsS "$@"; }
has() { [[ "$changed" == *" $1 "* ]]; }

wait_for() { # name, command...
  local name=$1; shift
  for _ in $(seq 1 60); do
    if "$@" >/dev/null 2>&1; then echo "  $name ready"; return 0; fi
    sleep 2
  done
  echo "::error::$name did not become ready" >&2
  return 1
}

# A setting as Compose sees it: the environment, else .env (read by key;
# never sourced).
setting() {
  local value
  value=$(printenv "$1" || true)
  if [ -z "$value" ] && [ -f .env ]; then
    value=$(grep -E "^$1=" .env | tail -1 | cut -d= -f2-)
    value=${value#[\"\']}; value=${value%[\"\']}
  fi
  printf '%s' "$value"
}

# ─── Preflight ───────────────────────────────────────────
# Alloy reads container logs from the systemd journal, so the stack needs a
# systemd Linux Docker host logging to journald, with persistent journal
# storage. Checked before anything changes. The paths are checked on the
# Docker host, which isn't always this machine (colima, a remote daemon):
# a throwaway container mounts each one strictly, which fails when it's
# missing and creates nothing. (They're variables only so a test can point
# them elsewhere; compose.yml mounts the real ones.)
machine_id=${MACHINE_ID_FILE:-/etc/machine-id}
journal_dir=${JOURNAL_DIR:-/var/log/journal}
log_driver=$(setting LOG_DRIVER)
channel=$(setting ALERT_CHANNEL)
on_docker_host() { # test flag, path
  docker run --rm --entrypoint test --mount "type=bind,src=$2,dst=/probe,readonly" \
    "$alloy_image" "$1" /probe >/dev/null 2>&1
}
problems=()
[ "${log_driver:-journald}" = journald ] ||
  problems+=("LOG_DRIVER is $log_driver: the observability stack needs journald (the default)")
[ -f "observability/grafana/channels/${channel:-none}.yml" ] ||
  problems+=("ALERT_CHANNEL=$channel is not one of: $(for f in observability/grafana/channels/*.yml; do basename "$f" .yml; done | tr '\n' ' ')")
if [ ${#problems[@]} -eq 0 ]; then
  # The first image named */alloy:*; awk reads to the end, so compose never
  # sees a closed pipe. A pull failure stops here with Docker's own error.
  alloy_image=$(obs config --images | awk '/\/alloy:/ && !n++')
  docker image inspect "$alloy_image" >/dev/null 2>&1 || docker pull -q "$alloy_image" >/dev/null
  on_docker_host -f "$machine_id" ||
    problems+=("the Docker host has no $machine_id: the observability stack needs a systemd Linux host (Docker Desktop can't run it)")
  on_docker_host -d "$journal_dir" ||
    problems+=("the Docker host has no $journal_dir: enable persistent journal storage there with 'sudo mkdir -p $journal_dir && sudo systemctl restart systemd-journald'")
fi
if [ ${#problems[@]} -gt 0 ]; then
  printf '::error::%s\n' "${problems[@]}" >&2
  echo "Nothing was changed. See docs/self-hosting/observability.md." >&2
  exit 1
fi

observability/scripts/ensure-monitor-role.sh -f "$COMPOSE_BASE" -f observability/compose.yml
grafana_before=$(obs ps -q grafana)
obs up -d alloy prometheus loki grafana

wait_for prometheus in_prom http://127.0.0.1:9090/-/ready
wait_for grafana http http://127.0.0.1:3000/api/health
wait_for loki http http://loki:3100/ready
wait_for alloy http http://alloy:12345/-/ready

# ─── Apply config changes ────────────────────────────────
if has grafana/channels && [ "$(obs ps -q grafana)" = "$grafana_before" ]; then
  # The alert channel is a single-file mount, which keeps showing the file
  # rsync replaced. A new container mounts the new one and provisions
  # everything as it starts. (Skipped when `up` just recreated Grafana:
  # every restart counts towards the crash-loop alert.)
  obs up -d --force-recreate grafana
  wait_for grafana http http://127.0.0.1:3000/api/health
fi
if has prometheus; then
  in_prom --post-data= http://127.0.0.1:9090/-/reload >/dev/null
  echo "  prometheus reloaded"
fi
if has alloy; then
  http -X POST http://alloy:12345/-/reload >/dev/null
  echo "  alloy reloaded"
fi
if has loki; then
  # Loki can't reload its main config; it's rarely touched.
  obs restart loki
  wait_for loki http http://loki:3100/ready
fi
if has grafana/provisioning; then
  # Dashboards reload from disk on their own; datasources, alerting and the
  # dashboard provider need an explicit reload. The break-glass admin's
  # password is already in the container's environment.
  for what in datasources alerting dashboards; do
    # shellcheck disable=SC2016 # expanded inside the container, on purpose
    obs exec -T grafana sh -c \
      'curl -fsS -X POST -u "admin:$GF_SECURITY_ADMIN_PASSWORD" "http://127.0.0.1:3000/api/admin/provisioning/$0/reload"' \
      "$what" >/dev/null
  done
  echo "  grafana provisioning reloaded"
fi

# ─── Verify ──────────────────────────────────────────────
# A failed reload keeps the last good config and says so only here.
check_flag() { # name, metric, command...
  local name=$1 metric=$2 value
  shift 2
  value=$("$@" | awk -v m="$metric" '$1 == m {print $2}')
  if [ "$value" != "1" ]; then
    echo "::error::$name: $metric is ${value:-missing}" >&2
    return 1
  fi
  echo "  $name config loaded"
}
check_flag prometheus prometheus_config_last_reload_successful in_prom http://127.0.0.1:9090/metrics
check_flag alloy alloy_config_last_load_successful http http://alloy:12345/metrics

obs ps alloy prometheus loki grafana
