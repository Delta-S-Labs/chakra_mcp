#!/usr/bin/env bash
# Self-hosting, end to end: follow docs/self-hosting/compose.md on a fresh
# machine and check each step a self-hoster takes.
#
#   CHAKRAMCP_IMAGE  the server image to test, e.g. chakramcp-server:e2e (required)
#   CHAKRAMCP_CLI    the chakramcp CLI binary (default: chakramcp on PATH)
#
# The stack runs on plain-HTTP names, app.localhost and relay.localhost: the
# CLI trusts only public certificate authorities, so Caddy's local one won't
# do. Everything else is the guide's: the same Compose file, `users add`,
# `chakramcp login` through the server's own pages, device pairing, an API
# key, and an MCP client that starts from the relay's URL alone.
# Used by .github/workflows/self-host-e2e.yml; runs locally too (Docker).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
infra=$(cd "$here/.." && pwd)
image=${CHAKRAMCP_IMAGE:?set CHAKRAMCP_IMAGE to the server image to test}
cli_bin=${CHAKRAMCP_CLI:-chakramcp}
work=$(mktemp -d)
export E2E_LOG_DIR="$work"
set -a
# shellcheck disable=SC1091  # test-only values next to this script
. "$here/fixtures.env"
set +a

app=http://app.localhost
relay=http://relay.localhost

compose() {
  docker compose -p chakramcp-e2e -f "$infra/docker-compose.prod.yml" --env-file "$work/.env" "$@"
}
# The CLI keeps its config under $HOME (macOS) or XDG_CONFIG_HOME (Linux):
# point both at the scratch directory so a real config is never touched.
cli() {
  HOME="$work/home" XDG_CONFIG_HOME="$work/home/.config" "$cli_bin" "$@"
}
step() { printf '\n== %s\n' "$*"; }
# `timeout` where there is one (Linux); macOS doesn't ship it.
with_timeout() {
  if command -v timeout >/dev/null; then timeout "$@"; else shift; "$@"; fi
}
fail() { printf 'FAILED: %s\n' "$*" >&2; exit 1; }

# Pair a device with `chakramcp pair` and approve it on the server's page as a
# new agent; the CLI ends up signed in with the device's token.
pair_and_approve() {
  local slug=$1 name=$2 out="$work/pair-$1.jsonl" pid url
  cli pair --json --no-open --display-name "$name" --agent-slug "$slug" > "$out" &
  pid=$!
  for _ in $(seq 1 30); do grep -q device_authorization "$out" 2>/dev/null && break; sleep 1; done
  url=$(python3 -c 'import json,sys; print(json.loads(open(sys.argv[1]).readline())["verification_uri_complete"])' "$out")
  E2E_AGENT_SLUG=$slug E2E_AGENT_NAME=$name python3 "$here/browser.py" pair "$url" \
    || fail "approving the pairing (see browser-pair.log)"
  wait "$pid" || fail "chakramcp pair didn't finish"
  grep -q '"event":"paired"' "$out" || fail "chakramcp pair didn't report the pairing"
  echo "paired: $(tail -1 "$out")"
}

finish() {
  status=$?
  if [ "$status" -ne 0 ]; then
    echo "--- relay and caddy logs"
    compose logs --tail 80 relay caddy 2>&1 || true
    for f in "$work"/browser-*.log "$work"/pair-*.jsonl; do
      [ -f "$f" ] && { echo "--- $(basename "$f")"; cat "$f"; }
    done
  fi
  compose down -v --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$work"
  exit "$status"
}
trap finish EXIT

step "1. Configure"
mkdir -p "$work/home"
cat > "$work/.env" <<EOF
APP_DOMAIN=$app
RELAY_DOMAIN=$relay
APP_PUBLIC_URL=$app
RELAY_PUBLIC_URL=$relay
POSTGRES_PASSWORD=$(openssl rand -hex 16)
JWT_SECRET=$(openssl rand -hex 32)
CHAKRAMCP_IMAGE=$image
LOG_DRIVER=json-file
EOF
echo "image $image"

step "2. Start"
compose up -d --wait pg redis relay caddy
for _ in $(seq 1 60); do
  curl -fsS "$relay/healthz" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS "$relay/healthz" && echo

step "3. Create the first admin"
printf '%s\n' "$E2E_ADMIN_PASSWORD" |
  compose exec -T relay chakramcp-server users add "$E2E_ADMIN_EMAIL" --name "$E2E_ADMIN_NAME" --admin --password-stdin

step "4. Point the CLI at the server"
cli networks add e2e --app-url "$app" --relay-url "$relay"
cli networks use e2e

step "5. Sign in with the CLI"
if [ "$(uname)" = Darwin ]; then
  # On macOS the CLI opens the default browser and ignores $BROWSER, so a
  # local run signs in by pairing instead. CI (Linux) takes the browser path.
  echo "macOS: signing in by device pairing"
  pair_and_approve e2e-login "E2E Login"
else
  BROWSER="$here/browser.py" HOME="$work/home" XDG_CONFIG_HOME="$work/home/.config" \
    with_timeout 120 "$cli_bin" login --method browser
fi
cli whoami | python3 -c '
import json, os, sys
me = json.load(sys.stdin)
assert me["user"]["email"] == os.environ["E2E_ADMIN_EMAIL"], me
print("signed in as", me["user"]["email"], "with", me["auth"])'

step "6. Create an API key for SDKs"
key=$(cli api-keys create --name e2e | python3 -c 'import json,sys; print(json.load(sys.stdin)["plaintext"])')
[ "$(curl -s -o /dev/null -w '%{http_code}' "$app/v1/me" -H "Authorization: Bearer $key")" = 200 ] \
  || fail "the new API key doesn't authenticate"
echo "the key authenticates"

step "7. Pair a device"
pair_and_approve e2e-agent "E2E Agent"

step "8. Connect as an MCP client"
python3 "$here/mcp_client.py" "$relay"

step "9. Self-hosted defaults"
status=$(curl -s -o "$work/signup.json" -w '%{http_code}' -X POST "$app/v1/auth/signup" \
  -H 'Content-Type: application/json' \
  -d '{"email":"stranger@example.test","password":"long-enough-password","name":"Stranger"}')
if [ "$status" != 403 ] || ! grep -q signup_disabled "$work/signup.json"; then
  fail "sign-up should be closed (got $status)"
fi
echo "public sign-up is closed"
compose exec -T relay chakramcp-server credits show "$E2E_ADMIN_EMAIL" --json | python3 -c '
import json, sys
view = json.load(sys.stdin)
assert view["status"] == "off" and view["enabled"] is False, view
print("credits are off")'

printf '\nAll steps passed.\n'
