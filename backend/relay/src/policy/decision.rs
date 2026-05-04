//! Decision types — the result of running the A2A policy gate.

use uuid::Uuid;

/// Outcome of `evaluate()`.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Authorized(Authorized),
    Denied(DenyReason),
}

/// All identifiers downstream code needs to actually forward the call
/// (D5). Stable across the policy result so the forwarder doesn't
/// need to re-query.
#[derive(Debug, Clone, PartialEq)]
pub struct Authorized {
    pub caller_user_id: Uuid,
    pub caller_account_id: Uuid,
    pub caller_agent_id: Uuid,
    pub target_account_id: Uuid,
    pub target_agent_id: Uuid,
    pub capability_id: Uuid,
    pub grant_id: Uuid,
    /// True if the target is in push mode (proxy). False = pull mode
    /// (park in inbox bridge).
    pub target_is_push: bool,
}

/// One of the structured failure modes. Each maps to a JSON-RPC error
/// code + a stable `data.code` string in the A2A error response. The
/// catalog at /.well-known/error-codes.json (D12) gives clients the
/// human-facing details and any deep-link URLs.
#[derive(Debug, Clone, PartialEq)]
pub enum DenyReason {
    AuthMissing,
    AuthInvalid,
    CallerAgentHeaderMissing,
    CallerAgentNotOwnedByCaller,
    CapabilityHeaderMissing,
    CapabilityHeaderInvalid,
    TargetAccountNotFound,
    TargetAgentNotFound,
    TargetTombstoned,
    CapabilityNotOnTarget,
    FriendshipRequired,
    GrantRequired,
    TargetUnreachable,
}

impl DenyReason {
    /// JSON-RPC `error.code`. Some reasons share a code (e.g. all
    /// auth failures map to -32000), with the discriminator carried
    /// in `data.code`.
    pub fn jsonrpc_code(&self) -> i32 {
        match self {
            // -32000: auth-required family.
            Self::AuthMissing | Self::CallerAgentHeaderMissing | Self::CapabilityHeaderMissing => {
                -32000
            }
            // -32001: malformed credentials.
            Self::AuthInvalid
            | Self::CallerAgentNotOwnedByCaller
            | Self::CapabilityHeaderInvalid => -32001,
            // -32002: friendship-required.
            Self::FriendshipRequired => -32002,
            // -32003: grant-required.
            Self::GrantRequired => -32003,
            // -32005: target unreachable.
            Self::TargetUnreachable => -32005,
            // -32006: target tombstoned (or so missing it might as well be).
            Self::TargetAccountNotFound
            | Self::TargetAgentNotFound
            | Self::TargetTombstoned
            | Self::CapabilityNotOnTarget => -32006,
        }
    }

    /// Stable `data.code`. Clients resolve display text + any
    /// deep-link URL via /.well-known/error-codes.json (D12).
    pub fn data_code(&self) -> &'static str {
        match self {
            Self::AuthMissing => "chk.auth.missing",
            Self::AuthInvalid => "chk.auth.invalid",
            Self::CallerAgentHeaderMissing => "chk.auth.caller_agent_header_missing",
            Self::CallerAgentNotOwnedByCaller => "chk.auth.caller_agent_not_owned",
            Self::CapabilityHeaderMissing => "chk.auth.capability_header_missing",
            Self::CapabilityHeaderInvalid => "chk.auth.capability_header_invalid",
            Self::TargetAccountNotFound => "chk.target.not_found",
            Self::TargetAgentNotFound => "chk.target.not_found",
            Self::TargetTombstoned => "chk.target.tombstoned",
            Self::CapabilityNotOnTarget => "chk.target.capability_unknown",
            Self::FriendshipRequired => "chk.policy.friendship_required",
            Self::GrantRequired => "chk.policy.grant_required",
            Self::TargetUnreachable => "chk.target.unreachable",
        }
    }

    /// Short human-readable summary for the JSON-RPC `error.message`.
    /// Don't put URLs here — those go in `data.code` → catalog.
    pub fn message(&self) -> &'static str {
        match self {
            Self::AuthMissing => "authentication required",
            Self::AuthInvalid => "invalid token",
            Self::CallerAgentHeaderMissing => "caller agent header required",
            Self::CallerAgentNotOwnedByCaller => "caller agent not owned by authenticated user",
            Self::CapabilityHeaderMissing => "capability header required",
            Self::CapabilityHeaderInvalid => "capability header is not a valid UUID",
            Self::TargetAccountNotFound => "target account not found",
            Self::TargetAgentNotFound => "target agent not found",
            Self::TargetTombstoned => "target agent has been deleted",
            Self::CapabilityNotOnTarget => "capability does not belong to target agent",
            Self::FriendshipRequired => "friendship required",
            Self::GrantRequired => "grant required",
            Self::TargetUnreachable => "target unreachable",
        }
    }
}
