# Waking pull agents W2: the SDKs wait — implementation plan

Derived from `2026-10-28-waking-pull-agents-design.md` (§6, "Clients"). W2 is PR 2 of §15:
the four SDKs use the relay's waiting pulls from W1 (#475).

**Branch:** `feat/w2-sdk-waiting-serve`.

## The contract, in every SDK

Each SDK uses its own spelling: `waitSecs`/`wait_s`/`WaitSecs`/`wait_secs`.

1. **Inbox `pull`.**
   - It keeps its return type and takes `wait` and `leaseSecs`.
   - The query sends `wait` and `lease_secs` only when they're set.
   - `lease_secs` is for the leases in W4. Older relays ignore it.
2. **`pullWithMeta`.**
   - It takes the same options and returns `{ items, waited }`.
   - `waited` comes from `ChakraMCP-Wait`. It's null or absent when the header is missing or isn't an integer, which is the case for relays before 0.7.
3. **Notices** take `wait`.
4. **Calls** get `pull` with `wait`, plus `pullWithMeta`.
5. **Inbox `serve`.**
   - It takes a wait setting (default 25) and a lease setting, which it passes to the pull.
   - The wait is capped to the whole seconds left until the next notices check (at least 1, rounded up), so notices keep their interval.
   - An empty pull goes straight back to pulling when `waited > 0`. Otherwise the loop sleeps the poll interval, as before.
   - Errors back off as before.
6. **Calls `serve`** gets the same wait setting and the same rule.
7. **`Notice`** gains an optional `invocation_id`.

## Per SDK

- **TypeScript:**
  - The options are `PullOptions { limit, wait, leaseSecs, signal }` and `PullResult<T>`.
  - The loops pass their `AbortSignal` to the pull, so aborting cancels a held request.
  - `waitSecs: 0` polls.
- **Python (sync and async):**
  - The new names are `wait`, `lease_secs`, `pull_with_meta` and `PullResult`.
  - `serve(wait_s=25, lease_secs=None)`; `wait_s=0` polls.
  - A pull's read timeout is raised to cover `wait`, for callers who pass their own `httpx` client.
  - The sync loop lets a held pull finish on stop rather than cancelling it.
- **Go:**
  - `Pull` and `Notices` keep their positional `limit`. New `PullWithOptions`, `PullWithMeta` and `NoticesWithOptions` take options structs, like `New`/`NewWithOptions`.
  - `ServeOptions.WaitSecs`: 0 means 25 and a negative value means never wait, matching `NoticeInterval` in the same struct.
  - `Serve` stops only when its own context ends, so an `http.Client` timeout shorter than the wait is reported and retried instead of ending the loop.
- **Rust:**
  - `pull` takes `impl Into<InboxPullQuery>`, and `From<Option<u32>>` keeps `pull(id, Some(5))` compiling.
  - Notices and calls take `PullQuery`.
  - A waiting pull's timeout is at least wait + 30 s.
  - Calls `serve` drops a held pull when cancelled.

## Tests

In every SDK:
- `pull` sends `wait` and `lease_secs`;
- `pullWithMeta` reads the header, and gives null without it;
- both serve loops pull again at once while the relay waits, and sleep otherwise, including on `ChakraMCP-Wait: 0`;
- serve sends `wait=25` by default;
- cancelling a held pull stops the loop.

**Relay:** an in-process load check, 2,000 waiting pulls on the hub with no database (`inbox_wait::tests`).
