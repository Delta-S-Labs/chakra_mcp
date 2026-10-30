# Waking pull agents W4b: the SDKs' claims and deadlines — implementation plan

The SDK half of W4 (`2026-10-29-waking-w4-plan.md`). It gives the four SDKs what relay
0.7 added in W3 (#494) and W4 (#496).

**Branch:** `feat/w4b-sdk-claims`.

## In every SDK

1. **`release`.**
   - Each SDK gets `inbox.release(invocationId)`, which calls
     `POST /v1/invocations/{id}/release` and returns the invocation.
   - A refusal is the SDK's usual API error, carrying the relay's code
     (`chk.invocation.not_releasable`, `chk.invocation.already_settled`).
2. **`expires_in` on invoke**, in seconds. It's sent only when set.
3. **`Peer`**:
   - `{delivery, online, last_seen_at}`;
   - on the invoke response, and on `Invocation` (to the caller, while open).
4. **New `Invocation` fields:**
   - `expires_at`, `lease_until` and `deliveries`;
   - all optional, so older relays still parse.
5. **Notices:** `invocation_expired` is documented as a notice kind (`invocation_id`
   arrived in W2).

## Spelling

Each SDK keeps its own existing request style:

- **TypeScript:** `expires_in` on `InvokeRequest`, like `consent_terms` and `call_id`.
  `Peer` is a union on `delivery`.
- **Python:** an `expires_in` key on the `InvokeRequest` TypedDict, and a `Peer` TypedDict.
- **Go:**
  - `InvokeRequest.ExpiresIn` (`int`; 0 leaves the default);
  - `Peer` is a struct;
  - `Inbox().Release`;
  - `NoticeInvocationExpired`.
- **Rust:**
  - `InvokeRequest.expires_in: Option<u32>`;
  - `Peer` is a struct, so a future `delivery` still parses;
  - `inbox().release`.

## Tests

In each SDK:
- `release` posts `{}` to the right path and decodes the invocation;
- a 409 surfaces its code;
- invoke sends `expires_in` only when set, and decodes `peer`.

The READMEs get a short paragraph on leases, deadlines, `release`, `expires_in` and `peer`.
