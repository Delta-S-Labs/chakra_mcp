---
name: chakramcp-hermes
description: "Use when the user wants to set up, run, or operate a ChakraMCP pull-mode agent (like 'Hermes'). Covers registration on the relay, exposing capabilities, polling the inbox via cron or systemd, and connecting to other agents (including push-mode A2A gateways like OpenClaw). Keywords: ChakraMCP, Hermes, OpenClaw, A2A, pull-mode agent, inbox.serve, cron polling, agent registration, friendship, grant, relay, agent directory."
---

# ChakraMCP — operate a Hermes-style pull-mode agent

Use this skill when the user wants to:

- Stand up a new ChakraMCP-native agent that polls the relay (no public host).
- Wire that agent to a peer (another ChakraMCP agent OR an external A2A
  gateway like [openclaw-a2a-gateway][openclaw]).
- Run the agent on a schedule (cron / systemd timer / GitHub Actions).
- Surface the agent in the public directory at `/agents`.
- Verify the round-trip with an end-to-end invocation.

The reference implementation lives at
`examples/hermes-openclaw-demo/`. Read that first, then adapt.

[openclaw]: https://github.com/win4r/openclaw-a2a-gateway

---

## What you're building

A "pull-mode" agent has **no public HTTP endpoint**. It long-polls the
relay's `/v1/inbox` instead. Pull-mode is the right choice for agents
that:

- Run on a laptop, behind NAT, or as a cron job.
- Don't want to expose an inbound port.
- Are written in a language with a ChakraMCP SDK (Python or TypeScript).

A **push-mode** agent advertises a canonical A2A v0.3 Agent Card at
some `agent_card_url`. The relay fetches the card (D2d), normalizes
it, and forwards JSONRPC calls. Use push-mode when the agent already
runs an A2A server (openclaw-a2a-gateway, custom Node, etc.).

This skill focuses on **pull-mode**, but covers calling push-mode
peers since that's the common interop story.

---

## Prerequisites

Confirm before starting:

1. **Python 3.10+** (`python --version`).
2. **A reachable relay**:
   - Local dev: `task db:up && task dev:backend && task dev:relay`
     (app on :8080, relay on :8090).
   - Or: `chakramcp-server start` if installed via Homebrew.
3. **`DISCOVERY_V2=true`** on the relay if the user wants directory listing.
4. **`chakramcp` Python SDK** installed: `pip install chakramcp`.

If anything is missing, fix it before generating code.

---

## Step-by-step

### 1. Register the agent

The user will need a ChakraMCP account. If they don't have one, sign
them up via the SDK or `POST /v1/auth/signup`, then mint an API key.

Then register a pull-mode agent (no `agent_card_url`):

```python
from chakramcp import ChakraMCP

chakra = ChakraMCP(api_key=API_KEY, app_url=APP_URL, relay_url=RELAY_URL)

agent = chakra.agents.create({
    "account_id": ACCOUNT_ID,
    "slug": "hermes",                        # URL-safe, unique within the account
    "display_name": "Hermes",
    "description": "Answers free-form questions through the ChakraMCP relay.",
    "tags": ["demo", "qa", "pull"],
    "visibility": "network",                 # 'network' = listed at /agents
})
print("agent_id:", agent["id"])
```

`visibility="network"` puts it in the public directory. Use
`"private"` if the user wants it hidden from `/agents` but still
callable through grants.

### 2. Add capabilities

Capabilities are typed RPC endpoints. JSON Schema for both inputs
and outputs — the relay enforces these on every invocation.

```python
chakra.capabilities.create(agent["id"], {
    "name": "answer_question",
    "description": "Answer a free-form text question.",
    "input_schema": {
        "type": "object",
        "required": ["question"],
        "properties": {
            "question": {"type": "string", "minLength": 1, "maxLength": 2000},
        },
    },
    "output_schema": {
        "type": "object",
        "required": ["answer"],
        "properties": {"answer": {"type": "string"}},
    },
    "visibility": "network",
})
```

### 3. Friendship + grant (so peers can call you)

Two agents must be **friends** before either can call the other.
Friendship is symmetric. Grants are directional and per-capability.

```python
# Propose from your side. The peer must accept.
fship = chakra.friendships.propose({
    "proposer_agent_id": agent["id"],
    "target_agent_id": PEER_AGENT_ID,
    "proposer_message": "Want to integrate?",
})

# After the peer accepts (peer-side code, peer's API key):
# chakra_peer.friendships.accept(fship["id"], message="Sure.")

# Then issue grants. This direction: PEER calls ME.
grant = chakra.grants.create({
    "granter_agent_id": agent["id"],
    "grantee_agent_id": PEER_AGENT_ID,
    "capability_id": cap["id"],
})
```

### 4. Implement the inbox loop

Two flavours — pick based on how the agent should run.

#### 4a. Long-running serve (development, daemons)

```python
import asyncio, signal
from chakramcp import AsyncChakraMCP

async def handle(invocation: dict) -> dict:
    inputs = invocation.get("input_preview") or {}
    if invocation["capability_name"] == "answer_question":
        return {
            "status": "succeeded",
            "output": {"answer": f"Echo: {inputs.get('question', '')}"},
        }
    return {"status": "failed", "error": "unsupported capability"}

async def main():
    chakra = AsyncChakraMCP(api_key=API_KEY, app_url=APP_URL, relay_url=RELAY_URL)
    stop = asyncio.Event()
    asyncio.get_event_loop().add_signal_handler(signal.SIGINT, stop.set)
    try:
        await chakra.inbox.serve(AGENT_ID, handle, poll_interval_s=1.0, stop_event=stop)
    finally:
        await chakra.aclose()

asyncio.run(main())
```

The relay attaches `grant_context` and `friendship_context` to
each invocation row — log them for audit, but don't re-verify;
the relay already did.

#### 4b. Cron / systemd / scheduled (production, no host)

For agents that don't run continuously, drain the inbox periodically:

```python
async def run_once():
    chakra = AsyncChakraMCP(api_key=API_KEY, app_url=APP_URL, relay_url=RELAY_URL)
    try:
        invocations = await chakra.inbox.pull(AGENT_ID, limit=25)
        for inv in invocations:
            try:
                result = await handle(inv)
                await chakra.inbox.respond(inv["id"], result)
            except Exception as err:
                await chakra.inbox.respond(inv["id"], {"status": "failed", "error": str(err)})
    finally:
        await chakra.aclose()
```

Wire it up:

**cron:**

```cron
* * * * * cd /opt/hermes && /opt/hermes/.venv/bin/python hermes_bot.py --once
```

**systemd timer** (`/etc/systemd/system/hermes.timer`):

```ini
[Unit]
Description=Drain Hermes inbox every minute

[Timer]
OnBootSec=30
OnUnitActiveSec=60
Persistent=true

[Install]
WantedBy=timers.target
```

With service unit (`/etc/systemd/system/hermes.service`):

```ini
[Unit]
Description=Hermes ChakraMCP agent — one-shot inbox drain

[Service]
Type=oneshot
ExecStart=/opt/hermes/.venv/bin/python /opt/hermes/hermes_bot.py --once
WorkingDirectory=/opt/hermes
EnvironmentFile=/opt/hermes/.env
```

**GitHub Actions** (`.github/workflows/hermes.yml`):

```yaml
on:
  schedule: [{ cron: "* * * * *" }]
jobs:
  drain:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - run: pip install chakramcp
      - run: python hermes_bot.py --once
        env:
          CHAKRAMCP_API_KEY: ${{ secrets.HERMES_API_KEY }}
```

### 5. Calling a push-mode peer (e.g. OpenClaw)

The wire shape is the same — your code never knows the peer is
push-mode. The relay handles the JWT mint + JSONRPC forwarding:

```python
result = await chakra.invoke_and_wait({
    "grant_id": GRANT_ID,                    # the grant that lets YOU call THEM
    "grantee_agent_id": YOUR_AGENT_ID,
    "input": {"ingredients": ["chicken", "rice", "lemon"]},
}, timeout_s=30)
```

The peer's Agent Card determines field mapping. For
openclaw-a2a-gateway the input dict gets stuffed into the A2A
SendMessage envelope's `parts[0].data`. Match the peer's published
input_schema and you're fine.

### 6. Verify in the directory

If `visibility=network` and `DISCOVERY_V2=true`, the agent shows up
at `http://localhost:3000/agents` (or your frontend host) within the
30-second revalidate window. Filter by tag, search by name, or hit
`/v1/discovery/agents?q=hermes` directly to confirm indexing.

---

## Connecting Hermes to OpenClaw end-to-end

Use the demo as a template:

```bash
cd examples/hermes-openclaw-demo
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

python mock_openclaw.py --port 18800 &     # tab 1
python setup.py                             # tab 2 (one-shot)
python hermes_bot.py &                      # tab 3

# Hermes calls OpenClaw (push-mode forwarder path):
python invoke_openclaw.py --ingredients chicken,rice

# OpenClaw calls Hermes (pull-mode inbox path):
python invoke_hermes.py --question "what is chakramcp?"
```

For real OpenClaw, replace `mock_openclaw.py` with the actual
[openclaw-a2a-gateway][openclaw] running on a public host, and pass
its `/.well-known/agent-card.json` URL as `--openclaw-card-url` to
`setup.py`. Nothing else changes.

---

## Common gotchas

- **`401 missing bearer token`** when calling a push-mode peer:
  the relay is reachable but couldn't mint a JWT. Check that
  `JWT_SIGNING_KEY` is set on the relay and that the peer's
  Agent Card lists `chakramcp_bearer` (or compatible) in
  `security_schemes`.
- **`404` from the directory page** for an agent you just registered:
  confirm `visibility=network` (not `private`), `DISCOVERY_V2=true`,
  and the row isn't tombstoned. Cache may be up to ~30s stale.
- **`hermes_bot.py --once` returns 1 with no logs**: a per-invocation
  handler raised. The script reports the failure back to the relay
  but exits non-zero so cron alerts fire. Add a `print(err)` in the
  except branch if you can't see what's wrong.
- **Friendship stuck in `pending`**: the peer hasn't accepted yet.
  Both sides must use their own API key for the accept call —
  the proposer cannot accept their own friendship.

---

## When NOT to use this skill

- The user has a public HTTP host already and wants A2A push-mode.
  Pull-mode is overkill. Use the Agent Card publishing path instead.
- The user wants to run the relay itself, not an agent against it.
  See `backend/relay/README.md`.
- The user wants to consume ChakraMCP from a non-Python/non-TS
  language. The relay's HTTP API works directly; this skill's
  Python-flavoured guidance won't add much.
