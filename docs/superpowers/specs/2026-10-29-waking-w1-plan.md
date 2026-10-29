# Waking pull agents W1: the hub and waiting pulls — implementation plan

Derived from `2026-10-28-waking-pull-agents-design.md` (§ numbers refer to it). W1 is PR 1
of §15. Each PR merges when green. The later PRs (W2–W7) get their own plans.

## Conventions

- **Branch:** `feat/w1-waiting-pulls`, cut from main.
- **Backend:**
  - Tests use `#[sqlx::test]`, placed next to the code they cover.
  - After any query change, run `cargo sqlx prepare --workspace -- --tests` with
    `DATABASE_URL=postgres://chakramcp:chakramcp@localhost:5544/chakramcp`.
  - Run `cargo clippy --workspace --all-targets -- -D warnings`.
- **Rule:** nothing on the invocation path may await new I/O. `notify` stays a plain
  `fn`.

## 1. The hub: `shared/src/wake/mod.rs` (§4)

### Types

- `pub enum Feed { Invocations, Notices, Calls }`. Wire codes are `"i" | "n" | "c"`.
- `#[derive(Clone)] pub struct Wake(Arc<Inner>)`, where `Inner` holds:
  - `process: Uuid`, a random id per `Wake`, used as the `o` field in §5's payloads;
  - `keys: Mutex<HashMap<(Uuid, Feed), watch::Sender<u64>>>`, the generation for each
    waited-on key;
  - `outbound: Mutex<HashSet<(Uuid, Feed)>>` and `outbound_ready: Notify`, for the bus;
  - `waiters: AtomicUsize` and `max_waiters: usize`;
  - `closed: watch::Sender<bool>`.

### API

- **`notify(&self, agent: Uuid, feed: Feed)`** is a plain function. It:
  - calls `signal_local`;
  - inserts the key into `outbound`;
  - calls `outbound_ready.notify_one()`;
  - increments `chakramcp_wake_notify_total`.
- **`signal_local(&self, agent, feed)`** bumps the key's generation (`send_modify`) if the
  key has a sender, and does nothing otherwise. The bus uses it for remote wakes.
- **`signal_all_local(&self) -> Vec<(Uuid, Feed)>`** returns the waited-on keys, for the
  bus's reconnect sweep.
- **`try_wait(&self, agent, feed) -> Option<Waiter>`** returns `None` when the waiter cap
  is reached or the hub is closed.
  - It gets or creates the key's sender and subscribes, which marks the current
    generation as seen.
  - It increments `waiters` and sets `chakramcp_inbox_waiters`.
  - `Drop for Waiter` decrements both, and removes the key once
    `sender.receiver_count()` would reach 0.
- **`Waiter::changed(&mut self, until: Instant) -> Woke`** returns
  `Woke::Changed | Woke::Deadline | Woke::Closed`. It is a `select!` over the key's
  `changed()`, `sleep_until`, and `closed.changed()`.
- **`close(&self)`** sets `closed`. Every waiter gets `Closed`, and `try_wait` returns
  `None` from then on.

### Settings

`WakeSettings::from_env()`, in the pattern of `outbound::OutboundPolicy::from_sources`:

| Setting | Default | Range |
| --- | --- | --- |
| `INBOX_MAX_WAIT_SECS` | 25 | 0–60 |
| `INBOX_MAX_WAITERS` | 2000 | ≥ 0 |

- An empty value counts as unset.
- A bad value stops startup.
- `0` turns waiting off: every pull answers with `ChakraMCP-Wait: 0`.

### Metric names

New entries in `chakramcp_shared::telemetry::names`:
- `INBOX_WAITERS` (gauge);
- `WAKE_NOTIFY_TOTAL`;
- `WAKE_LISTENER_RECONNECTS_TOTAL`.

### Unit tests

- A notify wakes a waiter (`Changed`).
- A notify between `try_wait` and `changed` is still seen, because the generation was
  marked at subscribe.
- Notifying a key nobody waits on creates no entry.
- The cap: once `max_waiters` is reached, `try_wait` returns `None`.
- `close` wakes waiters with `Closed`, and a later `try_wait` returns `None`.
- Dropping the last waiter removes its key.
- Three notifies of one key leave one outbound entry.

## 2. The bus: `shared/src/wake/bus.rs` (§5)

`pub fn spawn(pool: PgPool, wake: Wake)` starts two tasks.

### Sender

The loop:
1. wait for `outbound_ready` or 50 ms;
2. drain `outbound`;
3. build payloads `{"o":process,"w":[[agent,"i"],…]}` of at most 7 KB each;
4. send each one with `SELECT pg_notify('chakramcp_wake', $1)`.

On an error: log it at warn, put the keys back into `outbound`, and back off for 1 s.

### Listener

- It uses `PgListener::connect_with(&pool)`, then `listen("chakramcp_wake")`, then loops
  on `try_recv()`.
- **A message:** parse it and skip it if `o` is our own process. Call `signal_local` for
  each entry. Ignore unknown fields, so W6 can add `d`.
- **A bad payload:** log it at debug and skip it.
- **`Ok(None)`, the connection was lost:**
  - increment `WAKE_LISTENER_RECONNECTS_TOTAL`;
  - take `signal_all_local()` and signal the keys over about 1 s, in 20 slices
    50 ms apart;
  - continue, since `try_recv` reconnects.
- **`Err`:** log it, sleep with backoff (1 s up to 30 s), then reconnect.

### Tests (`#[sqlx::test]`)

- **Across two `Wake`s.** Build two `Wake`s on one pool and spawn a bus for each. A notify
  on A wakes a waiter on B within 2 s.
- **Echo.** A's own message doesn't signal A a second time. Check this through
  `WAKE_NOTIFY_TOTAL`, or by counting `signal_local` calls with a test hook.
- **Batching.** 500 keys split into payloads of 7 KB or less (a pure function test of the
  payload builder).

## 3. Wiring (§4, "Where it lives")

- **`AppState`** (`app/src/state.rs`) gets `pub wake: Wake` and `with_wake(wake)`.
  `new()` builds a fresh one, so tests need no change.
- **`RelayState`** (`relay/src/state.rs`) gets the same field and builder, plus
  `wait: WakeSettings`.
- **`server/src/main.rs`:**
  - build one `Wake::new(settings.max_waiters)`;
  - pass it to both states;
  - call `wake::bus::spawn(pool.clone(), wake.clone())` once;
  - at shutdown, call `wake.close()` right after `stop_tx.send(true)`, so waiters return
    inside `SHUTDOWN_DRAIN` (8 s).
- **`relay/src/main.rs` and `app/src/main.rs`:**
  - each builds its own `Wake` and bus;
  - the relay main adds `with_graceful_shutdown`, on Ctrl-C or SIGTERM, which calls
    `wake.close()`.
- **Startup log:** "inbox waits up to {n}s, {m} waiters max".

## 4. Where `notify` is called (§4 table, PR 1 rows)

Each call goes after the commit or statement that queues the work.

### Invocations

- `handlers/invoke.rs` `authorize_and_enqueue_trusted`: after the `INSERT … 'pending'`
  around line 1085. Wake `row.granter_agent_id`.
- `handlers/invoke.rs` `authorize_and_enqueue_public`: after the insert around line 1403.
  Wake `cap.granter_agent_id`.
- `inbox_bridge.rs` `park`: after the insert around line 245. Wake
  `authz.target_agent_id`.
- `shared/src/consent/decide.rs` `approve`:
  - it takes `wake: &Wake`;
  - after `tx.commit()`, if the release updated a row (use
    `RETURNING i.granter_agent_id` from the `released` CTE), wake that agent's
    `Invocations`;
  - update the callers: `relay/src/handlers/approvals.rs` around line 321, and
    `app/src/pages/approvals.rs` around line 132.

### Notices

- `relay/src/consent/enqueue.rs`: after the commit of the transaction holding the
  `consent_requested` insert (around line 165). Wake the granter agent.
- `relay/src/consent/expiry.rs` `expire_once`:
  - add `RETURNING agent_id` to the notice insert CTE and return those ids;
  - the worker wakes each one.
- `shared/src/consent/reply.rs` `notify`: it runs inside the caller's transaction, so the
  callers wake after their commit:
  - `confirm_now` takes `wake: &Wake` and wakes `t.granter_agent_id`. Its caller is
    `relay/src/consent/reply.rs` `submit_result`.
  - `decide::approve_reply` path (around 647) and `decide::deny` (around 719) take
    `wake: &Wake` and wake `d.granter_agent_id` when they sent a notice. Their callers
    are the relay approvals handler and the app approvals page.

### Calls

- Every `append(…, to)` site in `relay/src/calls/lifecycle.rs` (around 221, 273, 409, 505,
  585) and `messages.rs` (around 183, 193) wakes each agent in `to` with `Calls`, after
  that site's commit.
- `link.rs` passes `to = []`, so it wakes nothing.
- `lifecycle.rs` around 585 runs inside a caller's transaction, so its caller wakes after
  committing.
- `calls/workers.rs` `mark_missed`, `end_idle`, `end_gone`:
  - add `RETURNING agent_id` to the `told` CTEs, and return the recipients;
  - `tick` wakes them;
  - `spawn_workers` takes the `Wake`.

### Coverage test

A `#[sqlx::test]` per feed: start a waiting pull, perform the event, and assert that the
pull returns within 2 s. Events to cover:
- invoke (trusted, public, MCP, A2A park);
- approve a held request;
- a consent request (notice);
- a confirmed reply (notice);
- call invite, message, accept and end.

## 5. Waiting pulls (§6): `relay/src/inbox_wait.rs`

### The loop

```rust
pub async fn claim_waiting<T, F, Fut>(
    state: &RelayState, agent: Uuid, feed: Feed, wait_secs: Option<u32>, mut claim: F,
) -> Result<(Vec<T>, u32), ApiError>
where F: FnMut() -> Fut, Fut: Future<Output = Result<Vec<T>, ApiError>>
```

1. Clamp `wait_secs` to `state.wait.max_wait_secs`. If the result is 0, claim once and
   return `(items, 0)`.
2. Call `try_wait`. If it returns `None` (cap reached or closed), claim once and return
   `(items, 0)`.
3. Loop:
   1. claim, and return `(items, honoured)` if anything came back;
   2. otherwise wait on `waiter.changed(deadline)`:
      - `Changed`: loop again;
      - `Deadline`: return `([], honoured)`;
      - `Closed`: return `([], 0)` without claiming again.
4. Record a heartbeat on return. The start heartbeat already happens in each feed's
   authorization.

No pool connection is held between claims: each claim acquires one and releases it.

### One claim function for invocations

`relay/src/inbox_claim.rs`:
```rust
pub async fn claim_invocations(db, agent, limit, user_id) -> Result<Vec<Uuid>>
```
It's the claim SQL, used by REST `inbox` and MCP `pull_inbox`. MCP keeps its own
response shape.

### Splitting each feed into "authorize once" and "claim"

- **Invocations (REST `inbox`):** the agent lookup, membership and scope stay up front.
  The claim goes into the closure. Enrichment runs once, on the ids returned.
- **Notices:** `consent/notices.rs` `claim` splits into `authorize(state, user, agent)`
  and `claim_rows(db, agent, limit)`.
- **Calls:** `calls/feed.rs` `claim` splits into `act_as` plus heartbeat (once), then the
  claim CTE in the closure, then enrichment.

### REST

- `InboxQuery` and `FeedQuery` gain `wait: Option<u32>`.
- `inbox`, `inbox_notices` and the calls `inbox` handler return
  `([(HeaderName::from_static("chakramcp-wait"), honoured)], Json(..))`.
- The header is on every answer, including `wait=0`.

### MCP

- `pull_inbox`, `pull_notices` and `pull_calls` take `wait_seconds` (0–25) in their
  argument structs and in their tool schemas (`handlers/mcp.rs` tool list, and
  `calls/mcp.rs`).
- Description: "Wait up to this many seconds for new items before returning an empty
  list (0 = return at once; most this server honours: 25)."
- Results don't change shape.

### CLI

- `inbox pull`, `inbox notices` and `call pull` get `--wait <SECS>` (default 0), passed
  as `&wait=`.
- The CLI's 60 s timeout already covers a 25 s wait.
- `documented_commands.rs` covers the flags if it lists flag examples. Check, and add
  `inbox pull --agent x --wait 25`.

## 6. Docs in this PR

- **`docs/INSTALL.md` settings table:** rows for `INBOX_MAX_WAIT_SECS` and
  `INBOX_MAX_WAITERS`, plus a proxy note: "long-poll inbox requests hold up to 25 s; keep
  any proxy read timeout above that".
- **`.env.example`:** both settings, commented.
- **`charts/chakramcp/README.md`:** both settings in the extraEnv examples.
- **`frontend/src/app/(site)/docs/self-host/page.tsx`:** one bullet on the proxy timeout.
- **`/docs/api`:** check whether it documents `/v1/inbox` query parameters. If it does,
  add `wait` and the header there.

The skill and the agent guides wait for W7.

## 7. Tests (relay, `#[sqlx::test]`)

- **`wait=0`, or no `wait`:** today's behaviour, and the header says `0`.
- **`wait=5` on an empty inbox:**
  - an invoke made 200 ms later is returned within about 300 ms, and the header says `5`;
  - with no invoke, it returns `[]` after about 5 s. Use `tokio::time::pause` where the
    real DB allows it, or a short real wait such as `wait=1`.
- **Two waiting pullers, one invocation:** exactly one gets it, and the other returns `[]`
  at its deadline.
- **Clamping:** `wait=600` is clamped, and the header says `25`.
- **Waiter cap:** with `max_waiters = 1` and two pulls, the second answers at once with
  the header `0`.
- **`close()` during a wait:** the pull returns `[]` with the header `0`, and the row is
  still `pending`.
- **The same three cases on notices and calls.**
- **MCP:** `pull_inbox` with `wait_seconds` returns the invoked item.
- **Heartbeat:** a waiting pull records a heartbeat at the start and at the end. Check
  `heartbeats` in memory.
- **The coverage tests** from §4.

## 8. Verification and PR

1. Run `cargo test --workspace`, with Postgres on :5544 and `SQLX_OFFLINE=true` off
   locally, plus clippy and fmt.
2. **A hand check against a local `chakramcp-server`:**
   - in one shell, `chakramcp inbox pull --agent a --wait 25`;
   - in another, `chakramcp invoke …`;
   - the first returns at once.
3. **Measure:** run 200 waiting curls in a loop and record RSS and CPU. Note them in the
   PR.
4. **PR body:**
   - what changed;
   - the new settings;
   - that clients are unchanged until they send `wait`;
   - the measurements.
5. **After merging:** check this commit's CD run, then `chakramcp_inbox_waiters` and
   `chakramcp_wake_notify_total` in Prometheus.
