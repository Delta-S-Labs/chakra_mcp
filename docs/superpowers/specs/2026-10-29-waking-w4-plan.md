# Waking pull agents W4: claims and deadlines — implementation plan

Derived from `2026-10-28-waking-pull-agents-design.md` §9 ("Release", "Leases", "Late
answers"), §10 and §11–12. W4 is PR 4 of §15, split in two:

- **W4:** the relay, MCP, A2A and CLI.
- **W4b:** the SDKs, a separate PR after W4 merges:
  - `release`;
  - `expiresIn` on invoke;
  - the `peer` type promised in W3;
  - the new invocation fields.

**Branch:** `feat/w4-claims-deadlines`.

## Migrations

- **`0053_invocation_lease_columns`:**
  - columns `lease_until TIMESTAMPTZ`, `deliveries INTEGER NOT NULL DEFAULT 0` and
    `expires_at TIMESTAMPTZ`;
  - the `ACCESS EXCLUSIVE` lock is taken with 0046's retry loop (100 ms, 50 attempts);
  - the change is catalog-only.
- **`0054_invocation_lease_indexes`:**
  - `relay_invocations_open_expiry ON (expires_at) WHERE status IN ('pending','in_progress')`;
  - `relay_invocations_live_lease ON (lease_until) WHERE status = 'in_progress' AND lease_until IS NOT NULL`;
  - the `SHARE` lock is taken with 0047's retry loop.

## Settings (`relay/src/invocation_sweep.rs`, `InvocationSettings`)

| Setting | Default | Range |
| --- | --- | --- |
| `CLAIM_LEASE_SECS` | 600 | 30–86400 |
| `INVOCATION_DEFAULT_TTL` | `24h` | `<n>m`, `<n>h` or `<n>d`, 1 minute to 30 days |
| `INVOCATION_HITL_TTL` | `7d` | the same |

- They're read at startup like `WakeSettings`, and a bad value stops the boot.
- They live in `RelayState.invocations`.
- Each gets a row in `docs/INSTALL.md`, `.env.example`, the chart README and /docs/self-host.

## Claims

- **`claim_invocations`:**
  - `deliveries + 1`;
  - `lease_until = now() + lease` for every row except `human_in_loop` ones, which get
    NULL. The semantics are read in the `picked` CTE.
  - The lease is `lease_secs` from the pull, clamped to 1–86400, or `CLAIM_LEASE_SECS`.
- **Pulls take `lease_secs`:** REST `/v1/inbox` (the W2 SDKs already send it), MCP
  `pull_inbox` and CLI `inbox pull --lease-secs`.
- **`POST /v1/invocations/{id}/release`:**
  - **Auth:** as for results: membership of the called agent's account, and the
    credential's scope.
  - **Allowed:** an `in_progress` row with no pending reply draft. It goes back to
    `pending`: the claim fields and `lease_until` are cleared, and
    `deliveries = GREATEST(deliveries - 1, 0)`.
  - **Wake:** the agent's `Invocations` feed is woken.
  - **Refused:**
    - a settled row gets `409 chk.invocation.already_settled`;
    - a row that isn't claimed, or has a draft, gets `409 chk.invocation.not_releasable`.
  - **Clients:** MCP `release_invocation`, CLI `inbox release <id>`.
- **`inbox list --agent`** is a CLI alias for
  `invocations list --direction inbound --status pending --agent`.

## Results (`consent/reply.rs::submit_result`)

- **Accepted:**
  - an `in_progress` row;
  - a `pending` row with `deliveries > 0`: a late answer after its lease ran out. The
    first answer wins.
- **`human_in_loop` rows** must be `in_progress`. The owner's reply on pending rows comes
  in W5.
- **Refused:**
  - a `pending` row with `deliveries = 0` gets `409 chk.invocation.not_claimed`;
  - a settled row gets `409 chk.invocation.already_settled`;
  - `awaiting_consent` keeps today's 409.
- **The final `UPDATE`** checks `rows_affected`. Losing a race gets `already_settled`.

## Deadlines

- **At insert:** every `INSERT` sets `expires_at = now() + ttl`. That covers the
  trusted, public and consent enqueue paths, and A2A park.
- **The ttl:** the caller's `expires_in`, clamped to the default. Under 10 seconds is a
  400. The default is the default TTL, or the HITL TTL for `human_in_loop`.
- **`expires_in` on each surface:**
  - REST `POST /v1/invoke` `expires_in` (seconds);
  - MCP `invoke` `expires_in`;
  - A2A `params.metadata["chakramcp.expires_in"]`;
  - the CLI's `--expires-in 2h` on `invoke`.
- **Approval** (`shared::consent::decide::approve`) sets
  `expires_at = now() + (expires_at - created_at)` when it moves the row to `pending`.
- **Reads:** `expires_at`, `lease_until` and `deliveries` appear in REST invocation
  reads and inbox pulls (`InvocationDto`), and in MCP `poll_invocation`.

## The sweep (`relay/src/invocation_sweep.rs`)

Every 30 seconds, in every relay process (both binaries). Each pass works in batches with
`SKIP LOCKED`.

1. **Backfill.**
   - Open rows with no `expires_at` get `now() + ttl`, 1,000 at a time.
   - `awaiting_consent` rows are left alone.
2. **Deadlines.**
   - **Locking:** in one transaction, lock the pending reply drafts of due rows first
     (`FOR UPDATE SKIP LOCKED`). Then lock the due rows themselves, skipping any whose
     pending draft we didn't lock.
   - **The update:** each row becomes `timeout`. Its `error_message` is "picked up, never
     answered" when `claimed_at` is set, and "not picked up before its deadline"
     otherwise. `elapsed_ms` is set, and the locked drafts are `cancelled`.
   - **Notice:** an `invocation_expired` notice goes to the called agent (payload
     `invocation_id`), and its `Notices` feed is woken.
   - **Never waits:** every lock is `SKIP LOCKED`, so the sweep never waits on a lock.
   - **A late draft:** one stored after the second lock stays pending until the reply
     expiry worker expires it. Its release finds the row settled.
3. **Leases.**
   - An `in_progress` row past `lease_until` goes back to `pending`, with the claim
     fields cleared. Its agent is woken.
   - With `deliveries >= 3` it becomes `timeout` instead ("claimed 3 times, never
     answered"), and the agent gets `invocation_expired`.

**Metric:** `chakramcp_invocation_expired_total{reason}`, where the reason is
`not_picked_up`, `not_answered` or `lease_exhausted`.

## Notices

`Notice` gains `invocation_id`, from the payload. It's null when the payload has none.

## Tests

- **Migrations:** they apply, and the defaults are right (through `sqlx::test`).
- **Claims:**
  - a claim sets `deliveries` and `lease_until`, and NULL for `human_in_loop`;
  - `lease_secs` is clamped.
- **Release:**
  - it pends the row and wakes a waiting pull;
  - `deliveries` goes down;
  - it gives `not_releasable` on a pending row or one with a draft, and
    `already_settled` on a settled one;
  - the target side's scope is enforced.
- **Results:**
  - `not_claimed` on a fresh pending row;
  - a late answer on a lease-expired row lands;
  - a second answer gets `already_settled`;
  - the `rows_affected` race.
- **Deadlines:**
  - `expires_in` is clamped, and anything under 10 seconds is a 400;
  - the default TTLs by semantics;
  - approval restarts the clock;
  - the A2A metadata.
- **Sweep:**
  - backfill;
  - both timeout messages;
  - the notice and its `invocation_id`;
  - drafts are cancelled;
  - a draft locked by someone else skips the row;
  - leases pend and wake, and the third delivery times out;
  - the metric labels.
- **MCP:** `release_invocation`, `invoke` `expires_in`, `pull_inbox` `lease_secs`.
- **CLI:** the argument parsing and the documented-commands test.
