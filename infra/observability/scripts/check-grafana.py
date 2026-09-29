#!/usr/bin/env python3
"""Static checks for the Grafana provisioning and dashboards (observability CI).

- every provisioning file parses;
- dashboards and alert rules only use the provisioned datasource UIDs;
- alert rules don't write `$$`: Grafana doesn't unescape it in rules (only
  contact-point settings are env-expanded), so `$$1` / `$$labels` would reach
  Prometheus and the templates literally;
- rule and dashboard UIDs are unique;
- each alert channel (grafana/channels/, chosen by ALERT_CHANNEL) has its
  contact point under the uid `<channel>-chakramcp`, deletes every other
  channel's, and uses only message templates that are defined; `none`
  resets the policies and deletes them all.
"""
import glob
import json
import os
import re
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

defined = {name for doc in provisioned.values() for t in (doc or {}).get("templates", [])
           for name in re.findall(r'define "([^"]+)"', t.get("template", ""))}
channels = {os.path.basename(p)[:-4]: p for p in sorted(glob.glob(os.path.join(ROOT, "channels", "*.yml")))}
if "none" not in channels:
    fail("channels/none.yml is missing (ALERT_CHANNEL's default)")
for name, path in channels.items():
    try:
        doc = yaml.safe_load(open(path)) or {}
    except yaml.YAMLError as e:
        fail(f"{path}: does not parse: {e}")
        continue
    points = [(cp.get("name"), r.get("uid")) for cp in doc.get("contactPoints", []) for r in cp.get("receivers", [])]
    want = [] if name == "none" else [(name, f"{name}-chakramcp")]
    if points != want:
        fail(f"{path}: contact points should be {want}, got {points}")
    if name != "none" and [p.get("receiver") for p in doc.get("policies", [])] != [name]:
        fail(f"{path}: the root policy should route to {name!r}")
    if name == "none" and doc.get("resetPolicies") != [1]:
        fail(f"{path}: should reset org 1's policies")
    deleted = {d.get("uid") for d in doc.get("deleteContactPoints", [])}
    others = {f"{other}-chakramcp" for other in channels if other not in ("none", name)}
    if deleted != others:
        fail(f"{path}: should delete exactly the other channels' contact points {sorted(others)}, got {sorted(map(str, deleted))}")
    for used in re.findall(r'template "([^"]+)"', open(path).read()):
        if used not in defined:
            fail(f"{path}: uses template {used!r}, which provisioning/alerting doesn't define")

for msg in errors:
    print(f"::error::{msg}")
print(f"checked {len(provisioned)} provisioning files, {len(channels)} alert channels, {len(rule_uids)} alert rules, "
      f"{len(dash_uids)} dashboards: {'FAILED' if errors else 'ok'}")
sys.exit(1 if errors else 0)
