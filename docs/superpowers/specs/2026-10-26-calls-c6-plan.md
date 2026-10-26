# Calls C6: docs and switch-on — implementation plan

Derived from `2026-10-25-calls-design.md` (§ numbers refer to it); C1–C5 (#445–#450) shipped
the relay, the web, the SDKs and the CLI, dark. This is C6 of §17: the docs, calls on by
default, and production.

**Conventions:** branch `feat/calls-c6`, one PR, merged when green.

---

## Docs

- **`/docs/calls`** (new, under "Getting started" beside consent and safety): what a call
  is; who can call (`accept_calls`); states; the terms (`memory`, `ttl_secs`, `idle_secs`,
  `topic`); messages and their screening; the calls feed and `calls.serve`; capabilities in
  a call; call memory (the bundle, recall, what goes to TypeSafe); history and search;
  deletion; what owners do on the web; loop limits; errors; MCP, SDK and CLI in brief;
  self-hosting settings; limits.
- **Concepts:** a "Calls" section among the primitives.
- **CLI, SDK and MCP pages:** `chakramcp call`, the `calls` namespace and `calls.serve` in
  the four SDKs, and the MCP tools (`start_call` … `end_call`, `invoke` with `call_id`).
- **The autopilot guide** (step 4) and **the agent skill** (both copies,
  `.claude/skills/chakramcp-agent/SKILL.md` and `frontend/public/skills/chakramcp-agent.md`):
  how an agent answers and places calls.
- **llms.txt:** a calls section.
- **Safety page:** call messages in "What's screened"; it's on at chakramcp.com (since
  2026-10-24), so the "off by default" note says so.
- **Self-host page, README, `docs/system-one-compliance.md`:** the `CALLS_*` settings, and
  that call memory sends call content to TypeSafe.
- **Terms (the privacy note):** the relay now reads messages (safety screening) and keeps
  call transcripts until their TTL; call memory sends call content to TypeSafe. The
  "doesn't open them" line is no longer true. Version and date bumped.
- **SDK READMEs:** one line each on what call memory sends to TypeSafe.

## Gaps the docs exposed

Writing the CLI and autopilot docs showed two things a CLI-only agent couldn't do, so C6
adds them: `chakramcp call pull --as <agent>` (claims the calls feed, like `inbox pull`)
and `chakramcp agents update <id> --accept-calls …` (with `--name`, `--description`,
`--visibility`). MCP `update_agent` lists `accept_calls` in its schema (it already
accepted it).

## Calls on

- `CALLS_ENABLED` defaults to `true` in the relay's config, the Compose file, the
  `.env.example` files and the chart's notes. `CALLS_MEMORY_ENABLED` stays `false`: turning
  memory on in production is the owner's call, separately.
- Production's `.env` sets no `CALLS_*`, so the merge turns calls on there. Before
  merging, a smoke test on a local stack built from this branch: two accounts and agents,
  `chakramcp call start`, answer, send both ways, `show`, `end`, and an SDK `calls.serve`
  answering a call; the web's Calls tab and thread.
- After the deploy: `GET /v1/calls` answers 401 (not 404) without a token, the relay logs
  calls on, and the CD run is green.

## Verify

`pnpm lint`, `pnpm test`, `pnpm build` in `frontend`; the relay's config tests; the smoke
test above.
