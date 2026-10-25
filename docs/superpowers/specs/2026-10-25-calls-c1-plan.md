# Calls C1: core — implementation plan

Derived from `2026-10-25-calls-design.md` (§ numbers refer to it). This is C1 of §17: the
tables, the call lifecycle, the acting rules, messages with screening, the calls feed and
history, the background workers, and billing. Everything is dark behind `CALLS_ENABLED`
(default off).

**With the switch off, nothing changes:** no calls routes exist (404), no workers run, and
the only visible differences are the new migrations and `accept_calls` on agents.

**Conventions:** as in the safety plans.
- Branch `feat/calls-c1` (it already holds the spec).
- `cargo test --workspace` and clippy (`-D warnings`) before pushing.
- `cargo sqlx prepare --workspace -- --tests` for every new or changed query, with
  `DATABASE_URL` for `:5544` in the commit's shell.
- No token-shaped literals in tests (gitleaks).

**Refinements made while planning:**
- **`seq` comes from a counter on the call,** `calls.last_seq`, taken with
  `UPDATE calls SET last_seq = last_seq + 1 … RETURNING last_seq` in the entry's
  transaction. That serializes entries per call as the spec's row lock would, in one
  statement.
- **One ringing call per pair is a partial unique index,** `calls (caller_agent_id,
  callee_agent_id) WHERE state = 'ringing'`; a conflict answers 409 `chk.calls.busy`. No
  race between check and insert.
- **The per-sender message rate is counted in the database** (entries from that sender in
  that call in the last minute), not in Redis: exact, per call, and cheap on
  `(call_id, created_at)`.
- **`memory_features` is always `{"tags": false, "recall": false}` in C1.** The switch
  (`CALLS_MEMORY_ENABLED`) and the memory itself arrive in C3.
- **`relay_invocations.call_id` ships in C1,** because the purge clears those previews;
  C2 starts writing it. C1's tests set it directly.
- **`accept_calls` on agent DTOs is the effective value:** `nobody` for push agents
  whatever is stored. Agent cards gain it in C2, with the MCP work.
- **The `q` filter's OR query is a SQL function,** `calls_or_query(text)` in `0044`, which
  C3's recall reuses. Postgres makes the lexemes, so words in any script stay whole (a
  Rust split would cut Hindi words at their vowel signs).

**Refinements made while building:**
- **The interactive-session check stays in the relay,** as `auth::is_interactive`, used by
  the `InteractiveUser` extractor, consent replies and calls (step 2 had it moving to
  shared). It needs `RelayState`'s pool, and only the relay uses it.
- **Coded refusals:** `ApiError::Coded {status, code, message}` carries calls' `chk.calls.*`
  codes with their statuses; MCP maps it like `Policy`.
- **The Helm chart passes the calls settings through `extraEnv`,** as it does the credit
  amounts and the System One switches; `values.yaml` and the README list them.
- **Call charges ride in the invocation charge batch:** one transaction, one accounting
  lock, and the credits-off discard and the queue-depth gauge cover both queues.

---

## C1 steps

1. **Migrations** (`backend/migrations/`):
   - `0044_calls.sql` (cold, new tables): `calls` (§13, plus `last_seq INTEGER NOT NULL
     DEFAULT 0`), `call_entries`, `call_deliveries`, `call_charge_queue`, `call_charges`,
     with the §13 indexes and the ringing-pair unique index.
     - `call_entries.search` is `GENERATED ALWAYS AS (to_tsvector('simple'::regconfig,
       coalesce(text, ''))) STORED`, with a GIN index.
     - Agent foreign keys are nullable `ON DELETE SET NULL`; `call_deliveries` cascades
       from entries and agents.
     - The queue gets the credit queue's autovacuum settings.
   - `0045_agents_accept_calls.sql` (busy table, 0036's retry loop):
     `ADD COLUMN accept_calls TEXT NOT NULL DEFAULT 'friends'`, and its CHECK
     (`friends`, `anyone`, `nobody`) `NOT VALID`.
   - `0046_invocations_call_id.sql` (busy table, retry loop): `ADD COLUMN call_id UUID`.
   - `0047_invocations_call_id_index.sql` (retry loop): `CREATE INDEX IF NOT EXISTS
     relay_invocations_call_id ON relay_invocations (call_id) WHERE call_id IS NOT NULL`.
   - `0048_validate_accept_calls.sql`: `VALIDATE CONSTRAINT`.
   - A migration test (like `credits_schema_tests.rs`) checks the CHECKs, the unique
     ringing index and the cascades.

2. **Interactive sessions in shared:** `consent/reply.rs`'s private `interactive` moves to
   `shared/src/sessions.rs` as `pub async fn is_interactive(db, api_key_id, minted_jti)`,
   beside `is_delegated_token`; `consent/reply.rs` calls it. No behavior change.

3. **The `message` channel (shared):**
   - `safety::Channel::Message` (`"message"`), in `Channel::ALL`. The metrics test's
     series count grows by a third (108 → 144).
   - `questions::message_questions(subject)`: `off_relationship` when the relationship
     has terms (with the contacts-aside clause), and `prompt_injection` worded for "the
     agent receiving this message".
   - `safety_questions`: the per-contact "need" clause for `Message` is the note's ("rather
     than as part of what the message is about").
   - `ask()` adds `message_questions` for `Message`, as it adds `questions_in` for
     `Request`.
   - `block_for`: `Reply` and `Note` keep coding every block `safety_blocked`; `Request`
     and `Message` let harm win, else `compliance_denied`, with the reason "System One
     compliance check denied this message: …".
   - `ComplianceChecker::screen_message(text, data, relationship)`: screens
     `{"text": …, "data": …}` at root `message`.
   - Unit tests with the fake: a redacted message, a harmful one (`safety_blocked`), an
     `off_relationship` one (`compliance_denied`), `off_relationship` not asked without
     terms.

4. **Configuration:**
   - `relay/src/calls/config.rs`: `CallsConfig::from_env()` reads `CALLS_ENABLED`,
     `CALLS_MAX_TTL_DAYS`, `CALLS_DEFAULT_TTL_DAYS`, `CALLS_MAX_ACTIVE_PER_AGENT`,
     `CALLS_MAX_MESSAGES`, with the §14 defaults and range checks (the default TTL can't
     exceed the maximum). `RelayState` gains `calls: CallsConfig` and `with_calls`.
   - `CreditsConfig` gains `cost_per_message_mc` (`CREDITS_COST_PER_MESSAGE_MC`, default
     100), and `server.toml`'s credits section `cost_per_message_mc`.
   - Both `relay/src/main.rs` and `server/src/main.rs` build it and wire it.
   - Compose (`infra/docker-compose.prod.yml`, which self-hosters use too), both
     `.env.example` files and the Helm chart (`values.yaml`, `templates/deployment.yaml`'s
     env, the README table) gain the five `CALLS_*` variables and
     `CREDITS_COST_PER_MESSAGE_MC`, with defaults written out.

5. **Agents:**
   - `AgentDto` and `NetworkAgentDto` gain `accept_calls` (effective: `nobody` for push
     agents), in all four `AgentDto` queries and the network list.
   - `PATCH /v1/agents/{id}` accepts `accept_calls`: 400 for an unknown value; 409
     `chk.calls.push_unsupported` for a push agent and anything but `nobody`.

6. **The calls module** (`relay/src/calls/`), with thin axum handlers in
   `relay/src/handlers/calls.rs`, routes merged into the router only when
   `state.calls.enabled`:
   - `access.rs`:
     - `act_as(state, user, agent_id)`: the party rule of §4, which is membership of the
       agent's current account plus `resolve_grant`/`grant_allows_agent`, and refuses
       interactive sessions (403 `chk.calls.owner_session`);
     - `owner(state, user, call)`: an interactive session that's a member of either
       party agent's current account (403 `chk.calls.owner_only`);
     - `can_read(state, user, call)`: either party rule without the session check, or
       membership.
   - `invite.rs`, `POST /v1/calls`:
     1. The terms are validated (400 `chk.calls.bad_terms`), including calling yourself.
     2. The agents are checked: tombstoned (404), push mode (409).
     3. `accept_calls` (403 `chk.calls.not_accepted`): friends via an accepted
        friendship in either direction; same account unless `nobody`.
     4. The busy limits (409 `chk.calls.busy`).
     5. Usage limits on the caller's account (`enforce_limits(…, "calls")`).
     6. The topic is screened as a note; the opening message on the message channel,
        with the friendship as its relationship.
     7. In one transaction: the call (`ringing`), the `invite` entry, the opening
        `message` entry if any, the deliveries to the callee, the opening message's charge
        row, and an audit event `call.started`.
   - `answer.rs`: `accept` (with an optional first reply, screened and charged like a
     message) and `decline` (reason screened as a note), each a conditional `UPDATE …
     WHERE state = 'ringing'` (409 otherwise), with entries, deliveries (§6's table) and
     `call.answered` audit.
   - `end.rs`:
     - the party `end` and the owner `owner-end`, both conditional on `ringing` or
       `active`;
     - setting `ended_by`, `ended_at` and `purge_at`;
     - the `ended` entry and its deliveries (§6);
     - a `call.ended` audit event.
   - `messages.rs`, `POST /v1/calls/{id}/messages`:
     1. `act_as` and party membership of the call.
     2. The call must be `active` (409).
     3. 16 KiB on the serialized `{text, data}` (413).
     4. The per-sender rate (429 `chk.calls.slow_down`) and `CALLS_MAX_MESSAGES`
        (409 `chk.calls.full`).
     5. Usage limits.
     6. Screening when `safety_enabled()`.
     7. Blocked: a `blocked` entry with no content, then 403 with the code. Otherwise
        the entry, its delivery, its charge row and `last_message_at`/`message_count`,
        in one transaction. Returns `201` with the entry.
   - `feed.rs`, `GET /v1/inbox/calls`: `act_as` (as the inbox does), `heartbeats.record`,
     then claim at most 25 undelivered deliveries oldest first (`FOR UPDATE SKIP
     LOCKED`), mark them delivered, and join each with its entry and its call's summary
     (§6.1). No `memory` yet.
   - `read.rs`:
     - `GET /v1/calls`: with `agent_id`, that agent's calls under the party rule or
       membership; without it, all calls of agents in the user's accounts. Filters by
       state; cursor `(created_at, id)`.
     - `GET /v1/calls/{id}`.
     - `GET /v1/calls/{id}/messages?after=&limit=&q=`, with `q` through
       `search::or_query` (lexemes of three or more characters, ORed, ranked by
       `ts_rank_cd`).
   - `purge.rs`, used by the worker and by `DELETE /v1/calls/{id}/content`:
     - `close_invocations(conn, call_id, reason)`: cancels pending consent requests the
       way `consent/cascade.rs` does (new `cascade::cancel_for_invocations`), rejects held
       requests without charge, and fails `pending`/`in_progress` ones;
     - `purge_call(conn, call_id)`: clears entry content and tags, the invocations'
       previews and errors (keeping the closing error), and the call's topic and reason;
       sets `purged_at`; writes the `call.content_deleted` audit event.
   - DTOs in `mod.rs`: the call and entry objects of §11, with `safety` omitted when empty.

7. **Workers** (`relay/src/calls/workers.rs`), `spawn_workers(db, cfg)` called from both
   binaries when calls are enabled. A 30 s tick, `SKIP LOCKED` batches:
   - **missed:** ringing for 24 h, with the `missed` entry and delivery to the caller;
   - **ended:** active past `idle_secs`, and ringing or active with a tombstoned or deleted
     party (`ended_by` `idle` or `gone`), with entries and deliveries for both;
   - **purge:** calls past `purge_at` whose deliveries are done (or which ended more than
     24 h ago) and whose invocations are terminal. A call with unfinished invocations
     7 days past `purge_at` has them closed first.

8. **Credits:** the accounting pass gains `charge_call_rows`, which drains
   `call_charge_queue` into `call_charges` and debits wallets with each row's `cost_mc`,
   shaped like `charge_rows`. The credits-off `discard` drains it too. The queue-depth
   gauge reports both queues.

9. **Tests** (`#[sqlx::test]`, the fake TypeSafe), in `relay/src/calls/tests.rs`:
   - lifecycle: invite, accept with a first reply, decline, missed, idle end, party and
     owner end, tombstoned party; final states refuse everything with 409;
   - who can call: friends (each direction), anyone, nobody, same account, push agents,
     self, tombstoned; the busy limits and the ringing-pair index;
   - acting: an API key scoped to another agent (403), an interactive session on a party
     route (`owner_session`), an API key on owner routes (`owner_only`), reads for both;
   - messages: `seq` order under concurrent sends, 16 KiB, the rate and full limits;
     redacted and blocked messages with their codes and stored stand-ins;
   - feed: §6's recipients for every entry kind, each delivery claimed once, scoping;
   - history: `after`, `limit` and `q`;
   - billing: a charge row per delivered message and none for blocked ones; the worker
     debits them; credits off discards them;
   - purge: content gone and stub kept; waits for deliveries and for pending invocations
     (with `call_id` set directly); the 7-day close (consent requests cancelled, held
     requests rejected, the rest failed); owner deletion;
   - org deletion with calls on its agents, and a moved agent's calls still visible and
     billed to its new account;
   - `CALLS_ENABLED` off: every calls route 404s.

10. **Verify:** `cargo test --workspace`, clippy, the sqlx cache. Then open the PR with
    the spec, this plan and the code, and merge it when green.

## Open for C2's plan

The MCP tools; `invoke` and MCP `invoke` with `call_id` (writing
`relay_invocations.call_id`, the `invocation` entries read by join, `not_a_party`, the
`call_id` refusal when calls are off); `accept_calls` on agent cards.
