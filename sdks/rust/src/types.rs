use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationStatus {
    Pending,
    InProgress,
    Succeeded,
    Failed,
    Rejected,
    Timeout,
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
    pub i_granted: bool,
    pub i_received: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeResponse {
    pub invocation_id: String,
    pub status: InvocationStatus,
    pub error: Option<String>,
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
    /// Trust context bundled by the relay on `inbox.pull` responses
    /// only — the relay just verified friendship + grant before
    /// delivering this row, so handlers can trust these assertions
    /// without re-querying. `None` on audit-log endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendship_context: Option<FriendshipContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_context: Option<GrantContext>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct InvokeRequest {
    pub grant_id: String,
    pub grantee_agent_id: String,
    pub input: Value,
}
