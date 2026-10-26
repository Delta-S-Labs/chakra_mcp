# Calls C3: call memory with Jev — implementation plan

Derived from `2026-10-25-calls-design.md` (§ numbers refer to it); C1 (#445) and C2 (#446)
shipped the core and the MCP tools, dark. This is C3 of §17: call memory (§8): Jev
tagging, the memory bundle, recall, the switch, metering, and the memory eval.

**Two switches stay off:** `CALLS_ENABLED`, and `CALLS_MEMORY_ENABLED`, which turns memory
on only after the memory eval has run.

**Conventions:** as in C1. Branch `feat/calls-c3`.

**Refinements made while planning:**
- **The Jev logic lives in the shared crate,** `chakramcp_shared::calls_memory`, as pure
  functions:
  - building a tagging request and reading its answers;
  - building a recall request and reading its answer;
  - composing the bundle from tagged entries.

  The relay adds the database and HTTP plumbing. The memory eval runs the same functions
  on labelled transcripts, as `safety_eval` does for message safety.
- **Memory has its own TypeSafe client.** Today's client exists only when
  `SYSTEM_ONE_CHECKS` is on, and the spec says memory doesn't need the checks. A new
  `ComplianceChecker::client_from_env()` builds one from `TYPESAFE_AI_*` alone. The relay
  builds it when `CALLS_MEMORY_ENABLED` is on and keeps it as `RelayState.memory`.
  `memory_features` is `{tags, recall}` = whether that client exists.
- **A small log, `call_memory_requests`** (migration 0049), records each tagging and
  recall request: call, asking agent, kind, input tokens, cost. It serves both the
  per-agent recall limit and metering, and holds no content.
- **Tagging claims entries with a lease, not a lock.** Holding the call's row lock through
  a Jev request would make sends wait for it. A claim stamps `tagged_at` on pending
  entries (skipping those claimed in the last 5 minutes) and counts attempts in `tags`,
  so replicas don't tag twice and a crash only delays.
- **The bundle's items name senders by agent id,** like entries do, rather than "caller" or
  "callee".
- **`GET …/memory` and recall on a call without memory** answer 503
  `chk.calls.memory_unavailable` (an `own` call, or memory off when it was placed).
- **Recall's credit check uses the caller's account,** which pays for memory: `enforce_limits`
  on it, so an out-of-credits caller gets the usual 429 whoever asks.

---

## C3 steps

1. **Shared, `calls_memory`:**
   - **Types:** `Message {seq, from: Side, text, previous}`, `Open {seq, text}`, `Tags`
     (`decision`, `commitment`, `open_question`, `fact`, `weight`, `resolves`,
     `resolves_p`), `Candidate {seq, from, snippet}`.
   - **`tag_request(topic, &[Message], &[Open]) -> (state, questions)`:**
     - per message `m<seq>`: four nouls, a four-level `weight` score, and, when there
       are open questions, a `resolves` choice over `q<seq>` ids plus `none`;
     - texts sliced to 4 KiB, previous messages to 1 KiB, open questions to 300
       characters;
     - batches fitted to the 24 KiB state budget (`fit_batch`).
   - **`read_tags(&answers, &[Message]) -> Vec<(seq, Tags)>`.**
   - **`recall_request(question, &[Candidate]) -> (state, questions)`:** a `best` choice
     over `m<seq>` ids plus `none`, and an `answered` noul.
   - **`read_recall(&answers) -> (answered, Vec<(seq, p)>)`:** up to five, `p ≥ 0.05`.
   - **`bundle(&[BundleEntry], threshold, tags_on) -> Value`:** §8.3's composition and
     budgets (recent 8 KiB, pinned 6 KiB with `context` for short ones, open 2 KiB; recent
     only, 16 KiB, without tags).
   - **`ComplianceChecker`:**
     - `client_from_env()`;
     - `ask(&state, &questions) -> Result<Asked, CallError>` (`answers`, `input_tokens`,
       `model`), on `post`.
   - **The fake TypeSafe** answers `choice` and `score` questions, and takes an optional
     `answer` function for tests that need specific answers per question id.
   - **Unit tests:** the request shapes, reading answers, every bundle rule and budget.
2. **Migration 0049:** `call_memory_requests (id, call_id, agent_id, kind ('tag' |
   'recall'), input_tokens, cost_mc, created_at)`, with an index on `(call_id, agent_id,
   created_at)`.
3. **Config:** `CallsConfig` gains `memory_enabled` (`CALLS_MEMORY_ENABLED`),
   `tag_threshold` (`CALLS_TAG_THRESHOLD`, 0.7) and `memory_mc_per_1k_tokens`
   (`CALLS_MEMORY_MC_PER_1K_TOKENS`, 42). The two binaries build `RelayState.memory`.
   Compose, the `.env.example` files and the chart docs gain the three settings.
4. **Relay, `calls/memory/`:**
   - `invite` sets `memory_features` from `state.memory`.
   - Messages (invite, accept and send) are inserted with `tag_state = 'pending'` when
     `memory_features.tags` holds and they aren't blocked.
   - **`tagger.rs`, a background worker:**
     - per call with pending entries, claim a batch and load each message's previous
       text and the call's open questions;
     - ask Jev, store tags;
     - meter: a log row, a `memory` charge to the caller's current account, and
       `calls.memory_cost_mc`;
     - three failed attempts mark entries `error`;
     - calls whose caller is out of credits are skipped (the credit cache);
     - spawned beside the calls workers when memory is on.
   - **`bundle.rs`:** loads a call's messages and invocation entries (with previews by
     join) and composes. It's attached on the calls feed to the newest `message` item
     per memory call in each pull, and served at `GET /v1/calls/{id}/memory`.
   - **`recall.rs`, `POST /v1/calls/{id}/memory/ask`:**
     1. Check: a party, a memory call with recall on, not purged.
     2. Rate: 30 an hour per agent per call (429 `chk.calls.slow_down`).
     3. Credits: `enforce_limits` on the caller's account.
     4. Shortlist: full-text hits via `calls_or_query`, ranked, the best 48, with
        `ts_headline` snippets; then pinned and recent; at most 64.
     5. One Jev request, then the full stored text of the best five; metered and
        logged.
   - **MCP:** `call_memory` and `ask_call_memory`.
5. **The memory eval:**
   - **The labelled set,** `backend/shared/tests/data/calls/transcripts.json`: about 16
     calls, mostly English with Hindi, Bengali, Hinglish and Spanish ones. It has:
     - per message, its true tags and what it resolves;
     - per call, three recall questions with the `seq`s that answer them, and some with
       none.
   - **A fixture test** checks the set is well formed.
   - **`calls_memory_eval`** (`chakramcp-shared` example):
     - tags each call in batches, as the tagger does;
     - asks each recall question over the whole transcript as candidates;
     - prints per-tag precision and recall at the threshold, recall top-1 and top-3
       accuracy, "answered" calibration, and latency;
     - takes `--all` and `--threshold`, runs inside the relay container like
       `safety_eval`, and embeds the set.
6. **Tests** (`#[sqlx::test]`, the fake):
   - tagging stores tags, meters, and charges the caller;
   - a lease stops double tagging; three failures mark `error`;
   - an out-of-credits caller is skipped;
   - the feed attaches one bundle per memory call per pull;
   - `GET …/memory` composes pinned, open and recent from fake tags;
   - recall returns the chosen messages' full text, logs and charges, and is rate
     limited;
   - recall refused on `own` calls, with memory off, and on purged calls;
   - MCP `call_memory` and `ask_call_memory`;
   - calls placed with memory off keep features off and nothing pending.
7. **Verify:** `cargo test --workspace`, clippy, the sqlx cache; PR, merged when green.
   Then run the memory eval in the relay container, tune `CALLS_TAG_THRESHOLD`, and
   report. Memory stays off in production until the owner has seen the results.

## Open for C4's plan

The web: `/app/calls`, the thread view with the memory panel (`GET …/memory`), the
`accept_calls` setting, and call events in the audit log.
