# Owner consent PR 4: SDKs, examples and docs — implementation plan

Derived from `2026-10-23-owner-consent-design.md` §8 and §13; PRs 1–3 (#433–#436)
shipped the relay, the web pages and verified human-in-the-loop. Nothing is published
from this PR: the user wants one release after all four features (consent, safety,
calls, MCP) are built. So the SDK versions are bumped in the repo, and nothing is
tagged.

**Two PRs:**
- **4a:** SDKs and examples.
- **4b:** docs, prepared in parallel and reviewed before merging.

Both describe the behaviour already live since #436.

## 4a: SDKs and examples

1. **TypeScript (`sdks/typescript`, 0.5.0):**
   - **Types:**
     - `InvocationStatus` adds `awaiting_consent`;
     - `Invocation` gains `consent` and `human_confirmed`;
     - `InvokeRequest` gains `consent_terms`, and `InvokeResponse` gains
       `consent_request_id`;
     - `DraftedReply`;
     - `Notice` and its kinds;
     - capability `consent` settings and `semantics`.
   - **`inbox.respond()`** returns `Invocation | DraftedReply`.
   - **`inbox.notices(agentId, {limit})`.**
   - **`serve()`:**
     - polls notices every `noticeIntervalMs` (default 15 s) and calls
       `onNotice`. By default it logs a `consent_requested` notice's link.
     - `humanHandler` may now *return* a reply. The SDK posts it, and it becomes a
       draft. `onDraft(inv, drafted)` is called, and by default it logs the
       confirmation link.
     - Returning nothing keeps today's behaviour: the human answers in the web inbox.
   - **`verifyConsentToken(token, {jwksUrl | jwks, value?, now?})`:** checks `typ`,
     EdDSA through WebCrypto, `exp`, and the JCS SHA-256 of `value`.
   - **`canonicalJson(value)`** is exported.
   - **Tests:** the JCS vectors from `backend/relay/tests/data/jcs/vectors.json`, and
     a token signed with a test Ed25519 key (WebCrypto).
2. **Python (`sdks/python`, 0.5.0):** the same as TypeScript, in the sync and async
   clients.
   - **Verifying tokens** needs `cryptography` (a new optional extra,
     `chakramcp[verify]`), imported lazily with an install hint.
   - **JCS** is implemented in the SDK: keys sorted by UTF-16 code units, ES6 number
     formatting, and JSON escaping per RFC 8785.
   - **Tests:** the same vectors.
3. **Go (`sdks/go`, v0.3.0):**
   - the new statuses, fields, `DraftedReply` and notices types;
   - `Respond` returns `(*Invocation, *DraftedReply, error)` through a new
     `RespondResult`;
   - `Inbox.Notices`.
4. **Rust (`sdks/rust`, still 0.3.0, unpublished):** `consent` and `human_confirmed`
   on `Invocation`, `consent_terms` on invoke, a `RespondOutcome` enum (`Finished` or
   `Drafted`), and `inbox().notices()`.
5. **Examples:**
   - `examples/workers/pull-worker-with-hitl.ts` and `pull_worker_with_hitl.py`:
     `humanHandler` drafts a reply and logs the confirmation link.
   - `examples/voice-agent-demo`: drop `confirmed_by_human`, and show the link.

## 4b: docs

- **A concepts page,** `/docs/consent`: "Consent and verified human-in-the-loop",
  covering both gates, terms, standing consents, tokens and how to verify them, notices,
  self-hosted servers, and the limits (push agents).
- **Updated pages:**
  - `/docs/agents/step-3-capabilities` and `step-4-automation`;
  - `/docs/cli`, `/docs/mcp` and `/docs/sdk`;
  - the three example pages that mention `confirmed_by_human`.
- **Agent-facing text:** `frontend/public/llms.txt`,
  `.claude/skills/chakramcp-agent/SKILL.md`, and the host descriptor
  `frontend/public/.well-known/chakramcp.json` (the `message_owner` description).
- **READMEs:** `sdks/typescript/README.md`, `sdks/python/README.md`,
  `examples/workers/README.md` and `examples/voice-agent-demo/README.md`.
- **Template comments:** `cli/src/templates.rs` and the SDK template comments. Updating
  them together with the docs keeps the capability-templates hook happy.
