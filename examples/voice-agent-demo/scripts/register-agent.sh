#!/usr/bin/env bash
#
# register-agent.sh — one-time relay setup for a voice-agent persona.
#
# The relay's MCP surface is interaction-only (list/invoke/inbox/respond);
# it has NO agent-create or capability-publish tool. So the agent CANNOT
# register itself — that has to happen here, via the CLI, once per laptop
# before the demo.
#
# This registers the persona's agent (network-visible so the peer can
# discover it) and publishes the two capabilities the demo flow uses:
#   • message_owner   (reserved template, human-in-the-loop)
#   • negotiate_dinner (custom — the ranked-prefs negotiation RPC)
#
# Usage:
#   ./scripts/register-agent.sh kaustav      # persona name == personas/<name>.json
#   ./scripts/register-agent.sh aparajita
#
# Idempotent-ish: re-running after the agent exists will error on the
# duplicate slug — that's fine, it means you're already set up.

set -euo pipefail

PERSONA="${1:?usage: register-agent.sh <persona-name>}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PJSON="$HERE/personas/$PERSONA.json"

[ -f "$PJSON" ] || { echo "✗ no persona file at $PJSON" >&2; exit 1; }
command -v chakramcp >/dev/null || { echo "✗ chakramcp not on PATH — install the CLI first" >&2; exit 1; }

# --- pull persona fields (python3's json works on any 3.x) ---
read -r SLUG NAME DESC < <(python3 - "$PJSON" <<'PY'
import json, sys
p = json.load(open(sys.argv[1]))
print(p["agent_slug"], p["agent_display_name"], p.get("agent_description", ""))
PY
)
# DESC may contain spaces — re-read it whole.
DESC="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("agent_description",""))' "$PJSON")"

# --- pick the account: prefer an individual account, else the first ---
ACCOUNT="$(chakramcp whoami 2>/dev/null | python3 -c '
import json, sys
m = json.load(sys.stdin)["memberships"]
indiv = [x for x in m if x.get("account_type") == "individual"]
print((indiv or m)[0]["account_id"])
')"

echo "→ account:  $ACCOUNT"
echo "→ agent:    $SLUG  ($NAME)"

# --- 1. register the agent (network-visible for discovery) ---
chakramcp agents create \
  --account "$ACCOUNT" \
  --slug "$SLUG" \
  --name "$NAME" \
  --description "$DESC" \
  --visibility network

# --- 2. message_owner (reserved template, HITL) ---
chakramcp capabilities add \
  --agent "$SLUG" \
  --template message_owner

# --- 3. negotiate_dinner (custom RPC the peer invokes) ---
NEG_IN='{"type":"object","properties":{
  "from_agent":{"type":"string","description":"display name of the calling agent"},
  "round":{"type":"integer","description":"1-based negotiation round"},
  "their_drinks_ranked":{"type":"array","items":{"type":"string"}},
  "their_food_ranked":{"type":"array","items":{"type":"string"}},
  "notes":{"type":"string"}
},"required":["from_agent","their_drinks_ranked","their_food_ranked"]}'

NEG_OUT='{"type":"object","properties":{
  "agreed":{"type":"boolean"},
  "cuisine":{"type":"string"},
  "drink":{"type":"string"},
  "rationale":{"type":"string"}
},"required":["agreed"]}'

chakramcp capabilities add \
  --agent "$SLUG" \
  --name negotiate_dinner \
  --description "Negotiate a dinner cuisine + drink that respects both sides' ranked preferences." \
  --visibility network \
  --input-schema "$NEG_IN" \
  --output-schema "$NEG_OUT"

echo
echo "✓ $SLUG registered with message_owner + negotiate_dinner."
echo "  Verify:  chakramcp capabilities list --agent $SLUG"
