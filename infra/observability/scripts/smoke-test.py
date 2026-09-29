#!/usr/bin/env python3
"""Smoke-test a running observability stack (observability CI).

Run from the compose project directory after scripts/deploy.sh; every HTTP
call goes through `docker compose exec grafana curl`, so no ports need to be
published. Polls until everything holds or the deadline passes:

- metrics: each exporter reports real data (an exporter's own `up` stays 1
  even when its database login fails, hence pg_up / redis_up);
- one series behind each Postgres dashboard panel (catches a collector list
  that silently drops the defaults);
- logs: Loki holds Postgres's lines, i.e. the journald pipeline works, with
  only the `service` / `level` labels;
- Grafana: the three dashboards, the alert rules and the Telegram contact
  point are provisioned.
"""
import json
import subprocess
import sys
import time
import urllib.parse

COMPOSE = ["docker", "compose", "-f", "docker-compose.yml", "-f", "observability/compose.yml"]
DEADLINE = time.time() + 180


def curl(url, auth=False):
    cmd = COMPOSE + ["exec", "-T", "grafana", "sh", "-c",
                     'curl -fsS ${AUTH:+-u "admin:$GF_SECURITY_ADMIN_PASSWORD"} "$0"', url]
    if auth:
        cmd[len(COMPOSE) + 2:len(COMPOSE) + 2] = ["-e", "AUTH=1"]
    out = subprocess.run(cmd, capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or f"curl exited {out.returncode}")
    return json.loads(out.stdout)


def prom(query):
    return curl("http://prometheus:9090/api/v1/query?" + urllib.parse.urlencode({"query": query}))["data"]["result"]


def checks():
    """(name, ok) pairs for everything the stack should be doing."""
    yield "pg_up == 1", [r["value"][1] for r in prom("pg_up")] == ["1"]
    yield "redis_up == 1", [r["value"][1] for r in prom("redis_up")] == ["1"]
    for job in ["integrations/unix", "integrations/self", "prometheus", "loki", "grafana"]:
        yield f'up{{job="{job}"}} == 1', [r["value"][1] for r in prom(f'up{{job="{job}"}}')] == ["1"]
    for metric in [
        'pg_stat_database_xact_commit{datname="chakramcp"}',
        'pg_database_size_bytes{datname="chakramcp"}',
        "pg_locks_count",
        "pg_stat_activity_count",
        "pg_postmaster_start_time_seconds",
        'node_filesystem_avail_bytes{mountpoint="/"}',
        "node_memory_MemAvailable_bytes",
        "node_vmstat_oom_kill",
    ]:
        yield f"has {metric}", len(prom(metric)) > 0
    yield "no leaked __tmp labels", len(prom('{__tmp_root_disk!=""}')) == 0

    now = int(time.time())
    logs = curl("http://loki:3100/loki/api/v1/query_range?" + urllib.parse.urlencode(
        {"query": '{service="pg"}', "start": f"{now - 3600}000000000", "limit": 5}))["data"]["result"]
    yield "Loki has Postgres logs (journald pipeline)", any(s["values"] for s in logs)
    series = curl("http://loki:3100/loki/api/v1/series?" + urllib.parse.urlencode(
        {"match[]": '{service=~".+"}', "start": f"{now - 3600}000000000"})).get("data", [])
    labels = {key for s in series for key in s} - {"__stream_shard__"}
    yield f"Loki labels are service/level only (got {sorted(labels)})", labels <= {"service", "level"}

    for uid in ["chakramcp-overview", "chakramcp-infrastructure", "chakramcp-logs"]:
        yield f"dashboard {uid}", curl(f"http://127.0.0.1:3000/api/dashboards/uid/{uid}", auth=True)["dashboard"]["uid"] == uid
    rules = curl("http://127.0.0.1:3000/api/v1/provisioning/alert-rules", auth=True)
    yield f"alert rules loaded ({len(rules)})", len(rules) >= 12
    points = curl("http://127.0.0.1:3000/api/v1/provisioning/contact-points", auth=True)
    yield "telegram contact point", any(p["type"] == "telegram" for p in points)


while True:
    try:
        results = list(checks())
    except Exception as e:  # stack still starting
        results = [(f"query failed: {e}", False)]
    failing = [name for name, ok in results if not ok]
    if not failing:
        for name, _ in results:
            print(f"  ok  {name}")
        print("smoke test passed")
        sys.exit(0)
    if time.time() > DEADLINE:
        for name, ok in results:
            print(f"  {'ok  ' if ok else 'FAIL'}  {name}")
        print("::error::smoke test failed")
        sys.exit(1)
    time.sleep(10)
