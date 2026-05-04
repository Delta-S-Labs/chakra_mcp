//! Policy decision tree for A2A method calls.
//!
//! Every A2A call hitting `/agents/<acct>/<slug>/a2a/jsonrpc` runs
//! through this module before D5 lands the actual forward. The
//! decision tree is the single source of truth for "who can talk to
//! whom about what" — the trust IP that survived the migration from
//! the proprietary relay.
//!
//! Decision branches are listed in the order checked; the first
//! failure wins:
//!
//! 1. AuthMissing — no `Authorization: Bearer ...` header.
//! 2. AuthInvalid — bearer doesn't resolve to a user (JWT or API key).
//! 3. CallerAgentHeaderMissing — no `X-ChakraMCP-Caller-Agent`.
//! 4. CallerAgentNotOwnedByCaller — caller-agent doesn't belong to
//!    an account the authenticated user is a member of.
//! 5. CapabilityHeaderMissing — no `X-ChakraMCP-Capability`.
//! 6. CapabilityHeaderInvalid — header is not a UUID.
//! 7. TargetAccountNotFound — `<account-slug>` doesn't resolve to a
//!    live account.
//! 8. TargetAgentNotFound — `<agent-slug>` not under that account.
//! 9. TargetTombstoned — target row is tombstoned.
//! 10. CapabilityNotOnTarget — the capability_id header doesn't
//!     belong to the target agent.
//! 11. FriendshipRequired — no `accepted` friendship between caller
//!     agent and target agent.
//! 12. GrantRequired — no `active`, unexpired grant on
//!     (granter=target, grantee=caller, capability).
//! 13. TargetUnreachable — target is push-mode and its upstream card
//!     fetch hasn't succeeded recently.
//!
//! Pass: returns `Authorized { caller_agent_id, target_agent_id,
//! capability_id, grant_id }`. D5 picks this up and routes either to
//! the JWT-minter+forwarder (push) or the inbox bridge (pull).
//!
//! Notes:
//! - Consent is in the design spec but the schema has no
//!   consent_records yet. When that lands, a new `ConsentRequired`
//!   variant slots in between Grant and TargetUnreachable. Out of
//!   scope for D4.
//! - Health for "TargetUnreachable" uses the same threshold as the
//!   refresh job (`agent_card_fetched_at` >= now() - 4h is healthy
//!   for push; pull always passes since `inbox.serve()` polling
//!   keeps that side fresh — D5 handles parking-on-pull-stale).

pub mod decision;
pub mod evaluator;

pub use decision::{Authorized, Decision, DenyReason};
pub use evaluator::{
    evaluate, CALLER_AGENT_HEADER, CAPABILITY_HEADER,
};
