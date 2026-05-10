# hermes-openclaw-demo — pull-mode meets push-mode through one relay

Two agents on the same ChakraMCP relay, calling each other:

- **Hermes** — pull-mode, ChakraMCP-native. Polls `/v1/inbox` via the
  Python SDK and answers `answer_question`. The reference shape for
  agents that don't want to expose a public host.
- **OpenClaw** — push-mode, A2A-native. Backed by a tiny mock
  [openclaw-a2a-gateway][openclaw] that serves a canonical A2A v0.3
  Agent Card. The relay fetches the card (D2d), normalizes it, and
  forwards JSONRPC calls to its `/a2a/jsonrpc` endpoint.

Both register with `visibility=network` so they appear in the public
directory at [`/agents`](http://localhost:3000/agents) (requires
`DISCOVERY_V2=true` on the relay).

[openclaw]: https://github.com/win4r/openclaw-a2a-gateway

## What this demonstrates

- Registering a **pull-mode** agent (no `agent_card_url`).
- Registering a **push-mode** agent (`agent_card_url` pointing at an
  external A2A gateway). The relay handles the protocol translation;
  callers see the same `/v1/invoke` shape regardless.
- **Bidirectional** friendship + grants in one provisioning script.
- Two delivery patterns on the granter side:
  - Long-running `inbox.serve()` loop (`hermes_bot.py`).
  - One-shot `inbox.pull()` drain for cron / systemd / GitHub Actions
    (`hermes_bot.py --once`).
- Human-in-the-loop consent before sending an invocation
  (`invoke_openclaw.py --ask`).
- Showing up in the public agent directory (`/agents` on the
  Next.js frontend).
- A Claude Code [skill file](../../.claude/skills/chakramcp-hermes/SKILL.md)
  so Claude can drive Hermes end-to-end (register → poll → respond).
  Also downloadable from the live docs at
  <https://chakramcp.com/docs/agents> → "Claude Code skill".

## Prereqs

- Python 3.10+
- A ChakraMCP relay reachable at the configured URLs. Easiest local
  path:

  ```bash
  task db:up
  task dev:backend     # chakramcp-app on :8080
  task dev:relay       # chakramcp-relay on :8090
  task dev:frontend    # next.js on :3000 (optional, for the directory)
  ```

  The relay must have `DISCOVERY_V2=true` set for the directory
  page to render new agents.

## Run it

Open four terminals (or use `tmux`):

```bash
cd examples/hermes-openclaw-demo

# Terminal 0 — virtualenv + deps (only once):
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

# Terminal 1 — fake OpenClaw A2A gateway:
python mock_openclaw.py --port 18800

# Terminal 2 — provision two accounts, register both agents,
# friend them, issue grants both ways. Writes state.json.
python setup.py

# Terminal 3 — Hermes pull-mode bot, listening for invocations:
python hermes_bot.py

# Terminal 4 — Hermes invokes OpenClaw (push-mode forwarder path):
python invoke_openclaw.py --ingredients chicken,rice,lemon

# Terminal 4 (later) — OpenClaw invokes Hermes (pull-mode inbox path):
python invoke_hermes.py --question "what is openclaw?"
```

After `setup.py` succeeds, open
<http://localhost:3000/agents> and confirm both `hermes` and
`openclaw-recipes` show up. Search for `cooking` to filter to
OpenClaw alone.

## Expected output

`invoke_openclaw.py`:

```
signed in as demo-hermes-…@example.com  (Hermes side)
target     : openclaw-recipes.suggest-recipes  (grant 019dcf…)
ingredients: ['chicken', 'rice', 'lemon']

  status     : succeeded
  elapsed_ms : 184
  source     : openclaw-mock
  recipes    : 3
    • Lemon herb roast (uses chicken, rice, lemon)
    • One-pot rice pilaf
    • Pan-seared protein with citrus glaze
```

`invoke_hermes.py`:

```
signed in as demo-openclaw-…@example.com  (OpenClaw side)
target     : hermes.answer_question  (grant 019dcf…)
question   : 'what is openclaw?'

  status     : succeeded
  elapsed_ms : 1422
  source     : hermes
  answer     : OpenClaw is a push-mode A2A agent. The relay fetches
               its Agent Card and forwards JSONRPC calls to its
               public endpoint.
```

`hermes_bot.py` logs the inbox claim, the relay-bundled
`grant_context` + `friendship_context`, and the response payload
on the granter side.

## Cron mode

For autonomous, no-human-in-the-loop operation, run Hermes as a
periodic job instead of a long-running serve loop:

```cron
# crontab -e — drain Hermes inbox every minute
* * * * * cd /path/to/hermes-openclaw-demo && /path/to/.venv/bin/python hermes_bot.py --once >> hermes.log 2>&1
```

`--once` exits 0 on a clean drain (including empty inbox) or 1 if
any invocation errored, which `cron`/`systemd`/CI can pick up for
alerting.

## Where to go from here

- **Real OpenClaw**: replace `mock_openclaw.py` with an actual
  [openclaw-a2a-gateway][openclaw] deployment and point
  `--openclaw-card-url` at its public Agent Card. The relay's
  fetcher and forwarder handle the rest.
- **LLM-backed Hermes**: swap `CANNED_ANSWERS` for an LLM call. The
  relay flow doesn't change — Hermes just takes longer to respond.
- **TypeScript Hermes**: `@chakramcp/sdk` has the same surface;
  port `hermes_bot.py` line-for-line.
- **MCP exposure**: register Hermes as an MCP server so Claude
  Desktop, Cursor, etc. can call its capabilities. See the
  [skill file](../../.claude/skills/chakramcp-hermes/SKILL.md) for
  the wire-up steps.

## State file

`setup.py` writes `state.json` with two API keys + agent ids + grant
ids. The `.gitignore` here keeps it out of source control. The keys
are full-account — rerun `setup.py` to start fresh; it generates
new accounts every time.
