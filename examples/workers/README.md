# workers — copy-paste reference for pull-mode agents

Four files. Two languages. Two modes (autonomous and human-in-the-loop).
Each file is a single-process, long-running worker that uses
`inbox.serve()` to handle invocations off the ChakraMCP relay. Treat
them as starting points — clone the one closest to your shape, replace
the toy logic, ship.

These are the reference implementations for the HITL story landed
across issue [#69](https://github.com/Delta-S-Labs/chakra_mcp/issues/69)
(PRs 1–4 in `main`):

- DB column `agent_capabilities.semantics` (`autonomous` / `human_in_loop`) — PR #78
- Python SDK v0.3.0 routes HITL invocations to `human_handler` — PR #82
- TypeScript SDK v0.3.0 routes HITL invocations to `humanHandler` — PR #81

The relay no longer trusts a `confirmed_by_human: true` flag on the
result (the original gate, PR #79). Since owner consent (PR #436), a
reply to a `human_in_loop` capability from any agent credential (an
API key, an OAuth or device token, so the CLI and the SDKs too) is a
**draft**: the relay answers `202 {status: "awaiting_confirmation",
consent_request_id, approve_url, expires_at}`, and the caller sees
the reply only after one of the agent's owners confirms it on the web
at `approve_url`. See
[Consent and verified human-in-the-loop](https://chakramcp.com/docs/consent).

Plus the autonomous-orchestration sugar from cli-v0.1.2 (issue
[#68](https://github.com/Delta-S-Labs/chakra_mcp/issues/68)) — the HITL
workers below use `chakramcp invoke ensure --json` via subprocess to
show the outbound side without hand-rolling HTTP.

| File | Language | What it shows |
|---|---|---|
| [`pull_worker_autonomous.py`](pull_worker_autonomous.py) | Python | Single autonomous capability (`propose_slots`, `semantics: "autonomous"`). The SDK posts the handler's result back to the relay. |
| [`pull_worker_with_hitl.py`](pull_worker_with_hitl.py) | Python | Same agent also publishes `message_owner` (HITL). `human_handler` drops each pending invocation to `./pending/<id>.json`, drafts an acknowledgement for informational pings (`expects_reply: false`) and logs its confirmation link, and leaves everything else for the owner to answer. Bonus: outbound `chakramcp invoke ensure --json` via `subprocess.run()`. |
| [`pull-worker-autonomous.ts`](pull-worker-autonomous.ts) | TypeScript | TypeScript twin of the autonomous worker, using `@chakramcp/sdk@^0.5.0`. |
| [`pull-worker-with-hitl.ts`](pull-worker-with-hitl.ts) | TypeScript | TypeScript twin of the HITL worker. Outbound bonus via `node:child_process` `spawnSync`. |

## File-based HITL handoff

The HITL workers don't try to talk to the human directly. They write
each pending invocation to a directory. For an informational ping
(`expects_reply: false`) the handler returns an `acknowledged` reply:
the SDK posts it as a draft and `onDraft` / `on_draft` logs the link
the owner confirms it at. Otherwise the handler returns nothing, and
the row stays `in_progress`. A human (or any process that watches the
directory) picks it up and replies in the web app's inbox, or via the
CLI:

```bash
ls ./pending
# 01HXXX...json   01HYYY...json
cat ./pending/01HXXX...json | jq '.input_preview.message'
chakramcp message reply 01HXXX... "yes, friday at 2pm works"
```

`chakramcp message reply` builds the `message_owner`-shaped output JSON
and POSTs it. The relay saves it as a draft and the CLI prints:

```
Saved as a draft. One of the agent's owners confirms it before the caller sees it: https://chakramcp.com/app/approvals/…
```

An owner opens that link, signed in, and confirms (or rejects, with a
note). Only then does the row become `succeeded` and the original
caller's `invokeAndWait` / `invoke_and_wait` return the reply. A
rejected or unconfirmed draft fails the call ("The owner declined to
answer: …" or "Reply not confirmed in time"). An owner can also skip
the CLI and answer in the web app's inbox; an approver's answer there
needs no second step.

Returning a reply from `humanHandler` / `human_handler` (SDK 0.5.0)
works the same way: the SDK posts it, and it's a draft until an owner
confirms it.

This pattern is intentionally dumb. A real deployment likely replaces
the directory with a Slack DM, a pager, or an email — anything whose
human consumer eventually replies and confirms. The directory is the
lowest-common-denominator so the reference runs anywhere.

> **Future extension (not implemented):** a `--emit stdout` toggle for
> runtimes that prefer a stream over a directory (e.g. systemd journal,
> Kubernetes log collectors). Trivial to add — emit a single-line JSON
> per pending invocation on stdout in place of the file write, and let
> the human-facing tool tail the log. We left the file pattern in the
> reference because it's hermetic; bolt the stream on if you need it.

## Flow

```
   Autonomous worker:
   ────────────────────────────────────────────────────────────
        ┌──────────┐       ┌─────────┐       ┌──────────────┐
        │ inv pull │──────▶│ handler │──────▶│ respond(out) │──▶ relay
        └──────────┘       └─────────┘       └──────────────┘

   HITL worker:
   ────────────────────────────────────────────────────────────
        ┌──────────┐       ┌──────────────┐
        │ inv pull │──────▶│ humanHandler │──▶ ./pending/<id>.json
        └──────────┘       └──────────────┘                │
                                                           ▼
                                    ┌──────────────────────────┐
                                    │   human reads the file   │
                                    │            ▼             │
                                    │ chakramcp message reply  │
                                    │  <id> "<reply text>"     │
                                    └──────────────────────────┘
                                              │
                                              ▼
                                     relay: draft + approve_url
                                              │
                                              ▼
                                     owner confirms on the web
                                              │
                                              ▼
                                     caller gets the reply
```

## Run

### Python

```bash
cd examples/workers
python -m venv .venv && source .venv/bin/activate
pip install -r requirements.txt

export CHAKRAMCP_API_KEY=ck_…          # from `chakramcp keys create`
export CHAKRAMCP_AGENT_ID=01H…         # from `chakramcp agents create`
export CHAKRAMCP_PUBLISH=1             # first run only — publishes capabilities

# Autonomous-only worker:
python pull_worker_autonomous.py

# Or — autonomous + HITL:
python pull_worker_with_hitl.py
```

To exercise the outbound CLI demo in the HITL worker, also set:

```bash
export CHAKRAMCP_AGENT_SLUG=my-worker          # passed to `chakramcp invoke ensure --from`
export CHAKRAMCP_PING_PEER=other-account/their-agent
export CHAKRAMCP_PING_TEXT="hello from the reference worker"
python pull_worker_with_hitl.py
```

### TypeScript

```bash
cd examples/workers
npm install

export CHAKRAMCP_API_KEY=ck_…
export CHAKRAMCP_AGENT_ID=01H…
export CHAKRAMCP_PUBLISH=1

# Autonomous-only:
npm run worker:autonomous

# Autonomous + HITL:
npm run worker:hitl
```

Same `CHAKRAMCP_PING_PEER` / `CHAKRAMCP_AGENT_SLUG` env vars enable the
outbound `chakramcp invoke ensure --json` demo in the TS HITL worker.

### CLI prerequisites

The HITL workers shell out to `chakramcp` for both inbound resolution
(`chakramcp message reply <id> "<text>"`) and the outbound bonus
(`chakramcp invoke ensure …`). Install the CLI before running:

```bash
brew install chakramcp           # macOS / Linuxbrew
# or
cargo install chakramcp-cli      # from source
```

If `chakramcp` is missing from `PATH` the workers log a warning and
skip the outbound demo cleanly — the inbox loop still runs.

## See also

- [`examples/scheduler-demo/`](../scheduler-demo/) — the simpler
  end-to-end story (two autonomous Python agents calling each other
  through the relay). Read this first if you're new to the project.
- [`docs/agents`](https://chakramcp.com/docs/agents) — the canonical
  spec for capabilities, HITL semantics, and reserved templates.
- [`docs/consent`](https://chakramcp.com/docs/consent) — owner
  consent and verified human-in-the-loop: drafts, approvals, notices
  and consent tokens.
- Issue [#69](https://github.com/Delta-S-Labs/chakra_mcp/issues/69)
  — design spec for the HITL gate; this directory ships the PR-5
  reference closing it out.
