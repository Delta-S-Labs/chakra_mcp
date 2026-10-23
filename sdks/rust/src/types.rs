use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
    /// Visible to members of any organization-type account that shares
    /// membership with the owning account. Not in the public
    /// /v1/discovery/agents surface. Added with the org-settings work.
    Org,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FriendshipStatus {
    Proposed,
    Accepted,
    Rejected,
    Cancelled,
    Countered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantStatus {
    Active,
    Revoked,
    Expired,
}

/// An invocation's status. Non-exhaustive: the relay may add statuses, and
/// any it adds that this version doesn't know read as `Unknown` instead of
/// failing to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InvocationStatus {
    Pending,
    InProgress,
    Succeeded,
    Failed,
    Rejected,
    Timeout,
    /// Held until the target agent's owner approves it: the capability asks
    /// first. Becomes `Pending` on approval, `Rejected` if denied or expired.
    AwaitingConsent,
    /// A status this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

impl InvocationStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Rejected | Self::Timeout
        )
    }
}

#[allow(clippy::declare_interior_mutable_const)]
pub fn terminal_statuses() -> HashSet<InvocationStatus> {
    [
        InvocationStatus::Succeeded,
        InvocationStatus::Failed,
        InvocationStatus::Rejected,
        InvocationStatus::Timeout,
    ]
    .into_iter()
    .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountType {
    Individual,
    Organization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Member,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    pub account_id: String,
    pub slug: String,
    pub display_name: String,
    pub account_type: AccountType,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub user: User,
    pub memberships: Vec<Membership>,
    pub survey_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub account_id: String,
    pub account_slug: String,
    pub account_display_name: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub visibility: Visibility,
    pub endpoint_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_mine: bool,
    pub capability_count: i64,
    /// Mean of un-hidden review ratings (migration 0023). `None` when
    /// `review_count == 0`.
    pub avg_rating: Option<f64>,
    /// Count of un-hidden reviews.
    pub review_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub visibility: Visibility,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// True when the owner has opted this capability into being
    /// callable without a friendship/grant (migration 0022). When
    /// true, `public_monthly_quota_per_agent` carries the per-invoker
    /// monthly cap.
    pub public_invoke: bool,
    /// Per-invoker monthly quota. Some(_) only when `public_invoke`.
    pub public_monthly_quota_per_agent: Option<i32>,
    /// `"autonomous"` or `"human_in_loop"` (a reply is a draft until one
    /// of the agent's owners confirms it on the web).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<String>,
    /// Owner consent: whether requests wait for the owner's approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<CapabilityConsent>,
}

/// A capability's owner consent settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityConsent {
    pub required: bool,
    /// User ids who may decide; empty means the account's owners and
    /// admins. Only shown to those who manage the agent.
    #[serde(default)]
    pub approvers: Vec<String>,
    /// The longest standing consent: `"once"`, `"1h"`, `"1d"`, `"7d"` or `"30d"`.
    pub max_validity: String,
    /// How long a held request waits: `"<n>m"`, `"<n>h"` or `"<n>d"`.
    pub wait: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    pub account_id: String,
    pub account_slug: String,
    pub account_display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friendship {
    pub id: String,
    pub status: FriendshipStatus,
    pub proposer: AgentSummary,
    pub target: AgentSummary,
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
    pub counter_of_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub i_proposed: bool,
    pub i_received: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub status: GrantStatus,
    pub granter: AgentSummary,
    pub grantee: AgentSummary,
    pub capability_id: String,
    pub capability_name: String,
    pub capability_visibility: Visibility,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoke_reason: Option<String>,
    /// Why the granter issued this grant; `None` when unset.
    #[serde(default)]
    pub purpose: Option<String>,
    pub i_granted: bool,
    pub i_received: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeResponse {
    pub invocation_id: String,
    pub status: InvocationStatus,
    pub error: Option<String>,
    /// Set when the capability asks its owner first: the request is held
    /// (`AwaitingConsent`) or a standing consent covered it (`Pending`).
    #[serde(default)]
    pub consent_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invocation {
    pub id: String,
    pub grant_id: Option<String>,
    pub granter_agent_id: Option<String>,
    pub granter_display_name: Option<String>,
    pub grantee_agent_id: Option<String>,
    pub grantee_display_name: Option<String>,
    pub capability_id: Option<String>,
    pub capability_name: String,
    pub status: InvocationStatus,
    pub elapsed_ms: i32,
    pub error_message: Option<String>,
    pub input_preview: Option<Value>,
    pub output_preview: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub i_served: bool,
    pub i_invoked: bool,
    /// Which trust path authorised this invocation. `"friend"` = rode a
    /// friendship + grant (`friendship_context` / `grant_context` are
    /// populated on inbox responses). `"public"` = rode a
    /// `public_invoke=true` capability (migration 0022); contexts come
    /// back `None`. Use this instead of inspecting `grant_id` or the
    /// contexts directly — it removes the "None means public" footgun
    /// for handlers. Defaults to empty when consuming responses from
    /// relays predating PR #145.
    #[serde(default)]
    pub tier: String,
    /// Trust context for this invocation. Two sources:
    ///
    ///   - On `inbox.pull` responses, the relay just verified
    ///     friendship + grant before delivering this row — handlers
    ///     can trust these assertions without re-querying.
    ///
    ///   - On audit-log endpoints (`invocations.list/get`), for rows
    ///     queued on/after PR #146, the contexts come from a frozen
    ///     JSONB snapshot captured at queue time. They reflect what
    ///     was true when the call was queued, not necessarily what's
    ///     true now.
    ///
    /// `None` on public-invoke rows (no friendship/grant existed) and
    /// on legacy pre-snapshot rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendship_context: Option<FriendshipContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_context: Option<GrantContext>,
    /// The capability's semantics: `"human_in_loop"` means a reply is a
    /// draft until one of the agent's owners confirms it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantics: Option<String>,
    /// What the owner approved, for a capability that asks first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<InvocationConsent>,
    /// A person confirmed the reply, for a `human_in_loop` capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_confirmed: Option<HumanConfirmed>,
}

/// What a caller proposes when a capability asks its owner first. The
/// owner may approve it, narrow it or deny it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentTerms {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// `"once"` (default), `"<n>h"` (1-720) or `"<n>d"` (1-30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_for: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_requests: Option<u32>,
}

/// What the owner approved, and the consent token proving it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationConsent {
    pub consent_request_id: String,
    /// The standing consent that covered the request, if one did.
    pub consent_id: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub terms: ConsentTerms,
    /// An EdDSA JWT (`typ: "chakramcp-consent+jwt"`) signed with the
    /// relay's key, published at `{relay}/.well-known/jwks.json`; `None`
    /// until delivered.
    pub token: Option<String>,
}

/// A person confirmed the reply (`human_in_loop`), and the token proving it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanConfirmed {
    pub consent_request_id: String,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub token: Option<String>,
}

/// A reply to a `human_in_loop` capability, kept as a draft until one of
/// the agent's owners confirms it at `approve_url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftedReply {
    pub consent_request_id: String,
    pub approve_url: String,
    pub expires_at: DateTime<Utc>,
}

/// Something an agent is told out of band of its inbox. A
/// `consent_requested` notice means a call waits for your owner's
/// approval: pass `consent_request.approve_url` to them. The other kinds
/// are `reply_confirmed`, `reply_rejected` and `reply_expired`. Never
/// includes the call's input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_request: Option<NoticeConsentRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoticeConsentRequest {
    pub id: String,
    pub invocation_id: String,
    pub status: String,
    pub capability_id: String,
    pub capability_name: Option<String>,
    pub requester: Requester,
    pub terms: ConsentTerms,
    pub expires_at: DateTime<Utc>,
    pub approve_url: String,
}

/// The agent asking, and whether its account is verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requester {
    pub agent_id: String,
    pub agent_display_name: Option<String>,
    pub account_id: String,
    pub account_display_name: Option<String>,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendshipContext {
    pub id: String,
    pub status: FriendshipStatus,
    pub proposer_agent_id: String,
    pub target_agent_id: String,
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantContext {
    pub id: String,
    pub status: GrantStatus,
    pub granter_agent_id: String,
    pub grantee_agent_id: String,
    pub capability_id: String,
    pub capability_name: String,
    pub capability_visibility: Visibility,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Why the granter issued this grant; `None` when unset or when the
    /// relay/snapshot predates grant purposes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

// ─── Request bodies ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Default)]
pub struct CreateAgentRequest {
    pub account_id: String,
    pub slug: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateAgentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CreateCapabilityRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    /// Migration 0022. When `Some(true)`, the capability becomes
    /// callable by any registered agent without a friendship/grant —
    /// requires `visibility=Network` and a non-`None` quota.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_invoke: Option<bool>,
    /// Per-invoker monthly cap (calendar month). Required when
    /// `public_invoke` is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_monthly_quota_per_agent: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateCapabilityRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_invoke: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_monthly_quota_per_agent: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProposeFriendshipRequest {
    pub proposer_agent_id: String,
    pub target_agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposer_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CreateGrantRequest {
    pub granter_agent_id: String,
    pub grantee_agent_id: String,
    pub capability_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Optional free-text reason the grantee needs access. The relay
    /// trims it (empty becomes null) and caps it at 500 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Two flavours — exactly one of `grant_id` (trusted) or
/// `capability_id` (public-invoke, migration 0022) must be set. The
/// relay returns 400 if you send both or neither. Fill the rest with
/// `..Default::default()`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InvokeRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    pub grantee_agent_id: String,
    pub input: Value,
    /// For a capability that asks its owner first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent_terms: Option<ConsentTerms>,
}

// ─── Reviews (migration 0023) ──────────────────────────────

/// `friend` when the reviewer agent has an accepted friendship with
/// the target at write-time; `public` otherwise. Stamped on write,
/// won't drift later if the friendship state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewTier {
    Friend,
    Public,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewTag {
    pub capability_id: String,
    pub capability_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: String,
    pub reviewer: AgentSummary,
    pub target: AgentSummary,
    pub rating: i16,
    pub comment: Option<String>,
    pub tier: ReviewTier,
    pub tags: Vec<ReviewTag>,
    pub hidden: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// True when the caller owns the reviewer agent.
    pub i_authored: bool,
}

/// JSON keys are stringified bucket numbers, mirroring the relay's
/// `ReviewDistribution` shape. Use `Serialize`/`Deserialize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewDistribution {
    #[serde(rename = "1")]
    pub one: i64,
    #[serde(rename = "2")]
    pub two: i64,
    #[serde(rename = "3")]
    pub three: i64,
    #[serde(rename = "4")]
    pub four: i64,
    #[serde(rename = "5")]
    pub five: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSummary {
    /// Mean rating over the *visible* set. `None` when `count == 0`.
    pub average: Option<f64>,
    pub count: i64,
    pub distribution: ReviewDistribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewListResponse {
    pub reviews: Vec<Review>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub summary: ReviewSummary,
}

#[derive(Debug, Clone, Default)]
pub struct ReviewListQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub tier: Option<ReviewTier>,
    /// Owners-only flag. The relay silently force-clears it for
    /// non-target-account members so hidden reviews never leak.
    pub include_hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteReviewRequest {
    pub reviewer_agent_id: String,
    pub rating: i16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub tagged_capability_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EligibleReviewer {
    pub reviewer_agent_id: String,
    pub reviewer_display_name: String,
    pub tagable_capability_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EligibilityResponse {
    pub eligible: Vec<EligibleReviewer>,
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn statuses_parse_including_new_and_unknown_ones() {
        let parse = |s: &str| serde_json::from_value::<InvocationStatus>(serde_json::json!(s));
        assert_eq!(
            parse("awaiting_consent").unwrap(),
            InvocationStatus::AwaitingConsent
        );
        assert_eq!(parse("in_progress").unwrap(), InvocationStatus::InProgress);
        assert_eq!(parse("something_new").unwrap(), InvocationStatus::Unknown);
        assert!(!InvocationStatus::AwaitingConsent.is_terminal());
        assert!(!InvocationStatus::Unknown.is_terminal());

        let held: InvokeResponse = serde_json::from_value(serde_json::json!({
            "invocation_id": "i",
            "status": "awaiting_consent",
            "error": null,
            "consent_request_id": "c",
        }))
        .unwrap();
        assert_eq!(held.status, InvocationStatus::AwaitingConsent);
        assert_eq!(held.consent_request_id.as_deref(), Some("c"));
        let plain: InvokeResponse = serde_json::from_value(serde_json::json!({
            "invocation_id": "i", "status": "pending", "error": null,
        }))
        .unwrap();
        assert_eq!(plain.consent_request_id, None);
    }
}
