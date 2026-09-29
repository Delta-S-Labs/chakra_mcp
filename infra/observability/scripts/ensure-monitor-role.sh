#!/usr/bin/env bash
# Create the chakramcp_monitor role the collector's Postgres exporter logs
# in as: a member of pg_monitor (read-only statistics), nothing else.
# Idempotent. The password is sent only when the role is created — a
# failing statement is logged verbatim, and Postgres's logs go to Loki.
# Rotating it is a manual step (docs/CI-CD.md).
#
# Run from the compose project directory (/opt/chakramcp on the VM); any
# arguments are passed to `docker compose` (e.g. -f/--env-file in CI).
# PG_MONITOR_PASSWORD comes from the environment, else from the --env-file
# argument, else from ./.env (read by key; the file is never sourced).
set -euo pipefail

env_file=.env
prev=""
for arg in "$@"; do
  [ "$prev" = "--env-file" ] && env_file="$arg"
  prev="$arg"
done

compose=(docker compose "$@")
psql_pg() { "${compose[@]}" exec -T pg psql -U chakramcp -d chakramcp -v ON_ERROR_STOP=1 -qtA "$@"; }

if [ "$(psql_pg -c "SELECT 1 FROM pg_roles WHERE rolname = 'chakramcp_monitor'")" != "1" ]; then
  pw="${PG_MONITOR_PASSWORD:-}"
  if [ -z "$pw" ] && [ -f "$env_file" ]; then
    pw="$(grep '^PG_MONITOR_PASSWORD=' "$env_file" | tail -1 | cut -d= -f2-)"
  fi
  [ -n "$pw" ] || { echo "PG_MONITOR_PASSWORD is not set" >&2; exit 1; }
  case "$pw" in *"'"*|*\\*) echo "PG_MONITOR_PASSWORD must not contain quotes or backslashes" >&2; exit 1 ;; esac
  # Over stdin, so the password is never on a command line.
  printf "\\set pw '%s'\nCREATE ROLE chakramcp_monitor LOGIN PASSWORD :'pw';\n" "$pw" | psql_pg
  echo "created role chakramcp_monitor"
fi

# No password in these; safe to repeat on every deploy.
psql_pg <<'SQL'
SET client_min_messages = warning;
GRANT pg_monitor TO chakramcp_monitor;
ALTER ROLE chakramcp_monitor SET statement_timeout = '5s';
SQL
echo "chakramcp_monitor: ok"
