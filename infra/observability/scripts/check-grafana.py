#!/usr/bin/env python3
"""Static checks for the Grafana provisioning and dashboards (observability CI).

- every provisioning file parses;
- dashboards and alert rules only use the provisioned datasource UIDs;
- alert rules don't write `$$`: Grafana doesn't unescape it in rules (only
  contact-point settings are env-expanded), so `$$1` / `$$labels` would reach
  Prometheus and the templates literally;
- rule and dashboard UIDs are unique.
"""
import glob
import json
import os
import sys

import yaml

ROOT = os.path.join(os.path.dirname(__file__), "..", "grafana")
DATASOURCE_UIDS = {"prometheus", "loki"}
SPECIAL = {"-- Grafana --", "__expr__", "grafana", None}
errors = []


def fail(msg):
    errors.append(msg)


def datasource_uids(node):
    """Every datasource uid referenced anywhere in a JSON/YAML tree."""
    if isinstance(node, dict):
        ds = node.get("datasource")
        if isinstance(ds, dict):
            yield ds.get("uid")
        if "datasourceUid" in node:
            yield node["datasourceUid"]
        for value in node.values():
            yield from datasource_uids(value)
    elif isinstance(node, list):
        for value in node:
            yield from datasource_uids(value)


provisioned = {}
for path in sorted(glob.glob(os.path.join(ROOT, "provisioning", "**", "*.y*ml"), recursive=True)):
    try:
        provisioned[path] = yaml.safe_load(open(path))
    except yaml.YAMLError as e:
        fail(f"{path}: does not parse: {e}")

uids = [d.get("uid") for p, doc in provisioned.items() if "datasources" in p for d in doc.get("datasources", [])]
if set(uids) != DATASOURCE_UIDS:
    fail(f"datasources: expected uids {sorted(DATASOURCE_UIDS)}, got {uids}")

rule_uids = []
for path, doc in provisioned.items():
    for group in (doc or {}).get("groups", []):
        for rule in group.get("rules", []):
            rule_uids.append(rule["uid"])
            text = json.dumps(rule)
            if "$$" in text:
                fail(f"{path}: rule {rule['uid']} contains '$$' (written literally in rules; use a single $)")
            for uid in datasource_uids(rule):
                if uid not in DATASOURCE_UIDS | SPECIAL:
                    fail(f"{path}: rule {rule['uid']} uses unknown datasource uid {uid!r}")
if len(rule_uids) != len(set(rule_uids)):
    fail(f"duplicate alert rule uids: {sorted(u for u in rule_uids if rule_uids.count(u) > 1)}")

dash_uids = []
for path in sorted(glob.glob(os.path.join(ROOT, "dashboards", "*.json"))):
    try:
        dash = json.load(open(path))
    except json.JSONDecodeError as e:
        fail(f"{path}: does not parse: {e}")
        continue
    dash_uids.append(dash.get("uid"))
    for uid in datasource_uids(dash):
        if uid not in DATASOURCE_UIDS | SPECIAL:
            fail(f"{path}: unknown datasource uid {uid!r}")
if len(dash_uids) != len(set(dash_uids)) or None in dash_uids:
    fail(f"dashboard uids must be set and unique: {dash_uids}")

for msg in errors:
    print(f"::error::{msg}")
print(f"checked {len(provisioned)} provisioning files, {len(rule_uids)} alert rules, {len(dash_uids)} dashboards: "
      f"{'FAILED' if errors else 'ok'}")
sys.exit(1 if errors else 0)
