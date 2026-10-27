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
    /// Who may call this agent. Push agents always show `Nobody`. `None`
    /// from relays that predate calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_calls: Option<AcceptCalls>,
    /// Directory tags. Empty from relays before 0.6.0.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Who may call an agent. Non-exhaustive: a setting this SDK version
/// doesn't know reads as `Unknown` (don't send `Unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AcceptCalls {
    /// Agents it has an accepted friendship with (the default).
    Friends,
    /// Any agent that knows its id.
    Anyone,
    /// Invites are refused.
    Nobody,
    /// A setting this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

/// How an agent gets work. Non-exhaustive: a mode this SDK version
/// doesn't know reads as `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AgentMode {
    /// The relay calls its endpoint.
    Push,
    /// It pulls its inbox.
    Pull,
    /// A mode this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

/// An agent in the network directory ([`crate::ChakraMCP::network`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAgent {
    pub id: String,
    pub account_id: String,
    pub account_slug: String,
    pub account_display_name: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub mode: AgentMode,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether the owning account is verified.
    #[serde(default)]
    pub verified: bool,
    /// Set on your own agents only.
    pub endpoint_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_mine: bool,
    /// The capabilities on it that you can see.
    pub capability_count: i64,
    /// `None` when `review_count == 0`.
    #[serde(default)]
    pub avg_rating: Option<f64>,
    #[serde(default)]
    pub review_count: i64,
    /// Pulled its inbox in the last 5 minutes. `false` from relays before
    /// 0.4.0.
    #[serde(default)]
    pub online: bool,
    /// Who may call it. Push agents show `Nobody`. `None` from relays
    /// that predate calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_calls: Option<AcceptCalls>,
    // The shared directory fields: empty or `None` from relays before 0.6.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    /// The capabilities you can see, with who can call each.
    #[serde(default)]
    pub capabilities: Vec<CapabilityBrief>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub friend_count: i64,
    #[serde(default)]
    pub featured: bool,
}

/// A page of the network directory, newest first. Pass `next_cursor`
/// back as [`NetworkQuery::cursor`] for the next page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPage {
    #[serde(default)]
    pub agents: Vec<NetworkAgent>,
    /// `None` on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// `GET /v1/network/agents` paging.
#[derive(Debug, Clone, Default)]
pub struct NetworkQuery {
    /// 1 to 100; the relay's default is 20.
    pub limit: Option<u32>,
    /// `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

// ─── Discovery (relays from 0.6.0) ─────────────────────────

/// Who can call a capability: anyone signed in (`Public`, within its
/// monthly quota), or friends holding a grant (`Friends`).
/// Non-exhaustive: a value this SDK version doesn't know reads as
/// `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Access {
    Public,
    Friends,
    /// A value this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityBrief {
    pub id: String,
    pub name: String,
    pub access: Access,
}

/// The fields every directory list shares: search, featured, profiles
/// and the network list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentListing {
    pub id: String,
    pub slug: String,
    pub account_slug: String,
    pub display_name: String,
    pub description: String,
    pub mode: AgentMode,
    pub tags: Vec<String>,
    pub visibility: Visibility,
    /// Polled its inbox in the last 5 minutes. Push agents never show
    /// online.
    pub online: bool,
    /// Pull agents: the last inbox poll, else creation. Push: the last
    /// card fetch, else creation.
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub verified: bool,
    /// One of the relay's featured (house) agents.
    pub featured: bool,
    pub friend_count: i64,
    /// `None` when `review_count == 0`.
    pub avg_rating: Option<f64>,
    pub review_count: i64,
    pub accept_calls: AcceptCalls,
}

/// How you can call a capability: a grant to one of your agents, or
/// public invoke. Non-exhaustive, as [`Access`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CallableVia {
    Grant,
    Public,
    /// A value this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

/// A capability you can call now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallableCapability {
    pub capability: String,
    pub capability_id: String,
    pub via: CallableVia,
    /// The agent of yours holding the grant; `None` for a public
    /// capability, which any of your agents can call.
    pub as_agent_id: Option<String>,
}

/// A friendship between one of your agents and this one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingFriendship {
    pub id: String,
    pub your_agent_id: String,
    /// `Proposed` or `Accepted`.
    pub status: FriendshipStatus,
}

/// What you can do with an agent, for the agents your credential may act
/// as.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Standing {
    pub callable: Vec<CallableCapability>,
    pub friendships: Vec<StandingFriendship>,
    /// Your agents that could start a call with it.
    pub can_start_call_as: Vec<String>,
}

/// A search result ([`crate::DiscoveryClient::search`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryAgent {
    #[serde(flatten)]
    pub listing: AgentListing,
    pub capabilities: Vec<CapabilityBrief>,
    /// Deprecated: the same as `slug`.
    #[serde(default)]
    pub agent_slug: String,
    /// Deprecated: see `capabilities[].access`.
    #[serde(default)]
    pub has_public_capabilities: bool,
    /// Your standing, when the request is signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub you: Option<Standing>,
}

/// A page of search results. Pass `next_cursor` back as
/// [`DiscoveryQuery::cursor`], with the same filters, for the next page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPage {
    #[serde(default)]
    pub agents: Vec<DiscoveryAgent>,
    /// `None` on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// How many agents match. First page only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_estimate: Option<i64>,
}

/// `GET /v1/discovery/agents` filters. Every one is optional; fill the
/// rest with `..Default::default()`.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryQuery {
    /// Free text: any word matches, and better matches rank higher.
    pub q: Option<String>,
    /// Part of a capability's name.
    pub capability: Option<String>,
    /// A shape to match against any capability's output schema, e.g.
    /// `{"properties": {"slots": {}}}`.
    pub capability_schema: Option<Value>,
    pub mode: Option<AgentMode>,
    pub verified: Option<bool>,
    /// All of these tags.
    pub tags: Vec<String>,
    /// Also pull agents not seen in 30 days with nothing public.
    pub include_dormant: Option<bool>,
    /// Only agents you can call now. Needs a credential.
    pub callable: Option<bool>,
    /// 1 to 100; the relay's default is 20.
    pub limit: Option<u32>,
    /// `next_cursor` of the previous page.
    pub cursor: Option<String>,
}

/// An entry of the deprecated [`FeaturedAgent::public_capabilities`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturedCapability {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// An agent worth trying first ([`crate::DiscoveryClient::featured`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturedAgent {
    #[serde(flatten)]
    pub listing: AgentListing,
    pub capabilities: Vec<CapabilityBrief>,
    /// Deprecated: the same as `slug`.
    #[serde(default)]
    pub agent_slug: String,
    /// Deprecated: see `capabilities[].access`.
    #[serde(default)]
    pub public_capabilities: Vec<FeaturedCapability>,
}

/// A capability in full, as a profile shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub access: Access,
    /// Calls per calling agent per month, for a public capability.
    pub public_monthly_quota_per_agent: Option<i32>,
    /// Asks its owner before running.
    pub consent_required: bool,
    /// `"autonomous"` or `"human_in_loop"`.
    pub semantics: String,
}

/// The account that owns a profiled agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOwner {
    pub account_slug: String,
    pub display_name: String,
    pub verified: bool,
}

/// One of a profile's latest reviews.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileReview {
    pub rating: i16,
    pub text: Option<String>,
    pub at: DateTime<Utc>,
    /// The reviewer's display name, or "An agent" for a reviewer that
    /// isn't network-visible.
    pub by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileReviews {
    /// `None` when `count == 0`.
    pub avg_rating: Option<f64>,
    pub count: i64,
    /// Up to three, newest first.
    pub latest: Vec<ProfileReview>,
}

/// How to call a profiled agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HowToCall {
    /// A CLI command line.
    pub cli: String,
    /// Its A2A card (network-visible agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2a_card: Option<String>,
    /// Its page on the web (network-visible agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Its MCP endpoint, on relays that serve one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<String>,
}

/// An agent's profile ([`crate::DiscoveryClient::profile`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    #[serde(flatten)]
    pub listing: AgentListing,
    pub owner: AgentOwner,
    pub reviews: ProfileReviews,
    pub capabilities: Vec<CapabilityDetail>,
    pub how_to_call: HowToCall,
    /// Your standing, when the request is signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub you: Option<Standing>,
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
    /// Why a rejected call was refused, when message safety refused it:
    /// `chk.policy.safety_blocked` or `chk.policy.compliance_denied`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// What message safety removed, flagged or blocked in the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<Safety>,
}

/// One thing message safety found, and what it did. Never the removed
/// value: only its kind and where it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafetyFinding {
    /// `email`, `phone`, `url`, `messaging_link`, `handle`,
    /// `off_platform`, `opaque`, `unjudged`, or a question id such as
    /// `harm_crime`.
    pub kind: String,
    /// `redacted`, `flagged` or `blocked`.
    pub action: String,
    /// Where in the message, e.g. `input.note`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The question that judged it, when the model did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<f64>,
}

/// What message safety did to a call: `request` for the input, `reply` for
/// the reply (shown to the caller once it's released).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Safety {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Vec<SafetyFinding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Vec<SafetyFinding>>,
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
    /// What message safety removed, flagged or blocked in this call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<Safety>,
    /// The call this invocation was made in, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
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
    /// What message safety removed or flagged in the draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<Safety>,
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
    /// Directory tags: at most 10, each 1-32 lowercase letters, digits or
    /// hyphens. Relays from 0.6.0.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateAgentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    /// Who may call the agent. Push agents only take `Nobody`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept_calls: Option<AcceptCalls>,
    /// Replace the directory tags; `Some(vec![])` clears them. Relays
    /// from 0.6.0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
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
    /// Make the request inside this call: `grantee_agent_id` must be a
    /// party to the active call, and the capability the other party's
    /// (`chk.calls.not_a_party` otherwise).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

// ─── Calls ─────────────────────────────────────────────────

/// A call's state. Non-exhaustive: a state this SDK version doesn't know
/// reads as `Unknown` (don't send `Unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CallState {
    /// Invited, not yet answered.
    Ringing,
    Active,
    Declined,
    /// Still ringing 24 h after the invite.
    Missed,
    Ended,
    /// A state this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

impl CallState {
    /// `Declined`, `Missed` and `Ended` are final: the call is read-only.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Declined | Self::Missed | Self::Ended)
    }
}

/// Who remembers a call: `Call` (the relay keeps its memory) or `Own`
/// (each agent remembers for itself, the default). Non-exhaustive, as
/// [`CallState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CallMemoryMode {
    Call,
    Own,
    /// A mode this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

/// What ended a call. Non-exhaustive, as [`CallState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CallEndedBy {
    Caller,
    Callee,
    /// An owner of either agent, from the web.
    Owner,
    /// No messages for `idle_secs`.
    Idle,
    /// One of the agents was deleted.
    Gone,
    /// A value this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

/// A transcript entry's kind. Non-exhaustive, as [`CallState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CallEntryKind {
    /// The invite, with the caller's opening message, if any.
    Invite,
    Message,
    /// The callee accepted, with its first reply, if any.
    Accepted,
    /// The callee declined, with its reason, if any.
    Declined,
    Ended,
    Missed,
    /// A capability request made in the call (history only: the
    /// invocation reaches its agent through the inbox).
    Invocation,
    /// A kind this SDK version doesn't know.
    #[serde(other)]
    Unknown,
}

/// One side of a call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallParty {
    /// `None` once the agent is deleted.
    #[serde(default)]
    pub agent_id: Option<String>,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_slug: Option<String>,
}

/// What call memory can do on this call, fixed when it was placed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallMemoryFeatures {
    /// Messages are tagged, so memory holds `pinned` and `open`.
    pub tags: bool,
    /// The memory can be asked ([`crate::CallsClient::ask`]).
    pub recall: bool,
}

/// A call between two agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub state: CallState,
    pub caller: CallParty,
    pub callee: CallParty,
    pub memory: CallMemoryMode,
    pub memory_features: CallMemoryFeatures,
    /// How long the content is kept after the call ends.
    pub ttl_secs: u64,
    /// The call ends after this long with no messages.
    pub idle_secs: u64,
    #[serde(default)]
    pub topic: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_by: Option<CallEndedBy>,
    /// When the content will be deleted (set once the call is final).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purge_at: Option<DateTime<Utc>>,
    /// When the content was deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged_at: Option<DateTime<Utc>>,
    pub message_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
}

/// A page of calls, newest first. Pass `next_cursor` back as
/// [`ListCallsQuery::cursor`] for the next page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallList {
    pub calls: Vec<Call>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// The invocation an `invocation` entry stands for, as it is now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEntryInvocation {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<InvocationStatus>,
}

/// One entry in a call's transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEntry {
    /// Its place in the call, from 1.
    pub seq: i64,
    pub kind: CallEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_agent_id: Option<String>,
    /// `None` for a blocked message, events without text, and content
    /// past its TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Message safety blocked it: a stand-in with no content.
    pub blocked: bool,
    /// What message safety removed, flagged or blocked in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<Vec<SafetyFinding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<CallEntryInvocation>,
    pub created_at: DateTime<Utc>,
}

/// Jev's tags on a remembered message: the probability it's a
/// `decision`, `commitment`, `open_question` or `fact`, and its
/// `weight`, 0 (chit-chat) to 3 (an agreed term or deadline).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryTags {
    pub decision: f64,
    pub commitment: f64,
    pub open_question: f64,
    pub fact: f64,
    pub weight: f64,
    /// The `seq` of the earlier question this message answers, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolves: Option<i64>,
    /// How likely it answers that question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolves_p: Option<f64>,
}

/// An invocation held in a call's memory. `input` and `output` are set
/// once it's finished (a large one is `{"truncated": true, "bytes": n}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInvocation {
    pub id: String,
    pub capability: String,
    pub status: InvocationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

/// A message (verbatim, as stored) or an invocation in a call's memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub seq: i64,
    /// The sending agent's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// For a short message, the previous message's text ("Agreed." with
    /// what was agreed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<MemoryTags>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<MemoryInvocation>,
}

/// A call's memory (`memory: Call` only): `recent` messages and
/// invocations verbatim, plus, when the call's memory has tags, the
/// `pinned` decisions, commitments and facts from earlier and the `open`
/// questions not yet answered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallMemory {
    #[serde(default)]
    pub pinned: Vec<MemoryItem>,
    #[serde(default)]
    pub open: Vec<MemoryItem>,
    #[serde(default)]
    pub recent: Vec<MemoryItem>,
}

/// A message recall picked, with its full stored text and how likely it
/// answers the question (`p`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallMessage {
    pub seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub at: DateTime<Utc>,
    pub p: f64,
}

/// What asking a call's memory returns: up to five messages, best
/// first, and how likely any of them answers the question (`None` when
/// there was nothing to choose from). The agent reads them and answers
/// for itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecall {
    #[serde(default)]
    pub answered: Option<f64>,
    #[serde(default)]
    pub messages: Vec<RecallMessage>,
}

/// One item of an agent's calls feed: the call as it is now, the entry
/// delivered, and, in call-memory mode, the call's memory on the newest
/// message per call in a pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallFeedItem {
    pub call: Call,
    pub entry: CallEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<CallMemory>,
}

/// `POST /v1/calls`: invite `to_agent_id` to a call as `from_agent_id`.
/// Fill the rest with `..Default::default()`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StartCallRequest {
    pub from_agent_id: String,
    pub to_agent_id: String,
    /// `Own` when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<CallMemoryMode>,
    /// Content lifetime after the call ends; the relay's default (30
    /// days) when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
    /// 3,600 to 604,800; 86,400 (24 h) when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_secs: Option<u64>,
    /// At most 200 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The opening message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `GET /v1/calls` filters. With no `agent_id`, the calls of all your
/// agents.
#[derive(Debug, Clone, Default)]
pub struct ListCallsQuery {
    pub agent_id: Option<String>,
    pub state: Option<CallState>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// `GET /v1/calls/{id}/messages` options: entries after the `after` seq,
/// oldest first; with `q`, the entries matching any of its words, best
/// first.
#[derive(Debug, Clone, Default)]
pub struct CallHistoryQuery {
    pub after: Option<i64>,
    pub limit: Option<u32>,
    pub q: Option<String>,
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

        // Message safety's fields, when a call was screened.
        let refused: InvokeResponse = serde_json::from_value(serde_json::json!({
            "invocation_id": "i", "status": "rejected", "error": "blocked",
            "code": "chk.policy.safety_blocked",
            "safety": {"request": [{"kind": "harm_crime", "action": "blocked",
                                    "question": "harm_crime", "p": 0.93}]},
        }))
        .unwrap();
        assert_eq!(refused.code.as_deref(), Some("chk.policy.safety_blocked"));
        let finding = &refused.safety.unwrap().request.unwrap()[0];
        assert_eq!(
            (finding.kind.as_str(), finding.action.as_str()),
            ("harm_crime", "blocked")
        );
        assert_eq!(finding.path, None);
    }
}
