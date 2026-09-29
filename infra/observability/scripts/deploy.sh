#!/usr/bin/env bash
# Bring the observability stack up to date — run by CD on the VM, from the
# compose project directory (/opt/chakramcp), after the config has been
# synced into ./observability/.
#
#   observability/scripts/deploy.sh "<changed config dirs>"
#
# The argument lists the config directories whose content changed (CD
# hashes them before and after syncing): alloy, prometheus, loki,
# grafana/provisioning, grafana/dashboards. Changes are applied by reload
# wherever the service supports it, so a config deploy never shows up as
# a restart (the crash-loop alert counts restarts). Compose itself
# recreates a service whose definition or image changed.
set -euo pipefail

changed=" ${1:-} "
obs() { docker compose -f docker-compose.yml -f observability/compose.yml "$@"; }
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

observability/scripts/ensure-monitor-role.sh
obs up -d alloy prometheus loki grafana

wait_for prometheus in_prom http://127.0.0.1:9090/-/ready
wait_for grafana http http://127.0.0.1:3000/api/health
wait_for loki http http://loki:3100/ready
wait_for alloy http http://alloy:12345/-/ready

# ─── Apply config changes ────────────────────────────────
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
