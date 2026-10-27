//! Featured agents: the ones worth trying first. The featured account's
//! agents (the house agents), then online agents with something publicly
//! callable, up to 12.

use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::fields::{self, AgentSummary, CapabilityBrief};
use crate::state::RelayState;

/// How many agents are featured at most.
const FEATURED_LIMIT: i64 = 12;

/// A publicly callable capability, so a page can show a working call.
#[derive(Debug, Clone, Serialize)]
pub struct FeaturedCapability {
    pub id: Uuid,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeaturedAgent {
    #[serde(flatten)]
    pub agent: AgentSummary,
    pub capabilities: Vec<CapabilityBrief>,
    /// Deprecated, the same as `slug`. Kept for one release.
    pub agent_slug: String,
    /// Deprecated: see `capabilities[].access`. Kept for one release.
    pub public_capabilities: Vec<FeaturedCapability>,
}

/// The featured agents, loaded for `viewer`. A short list apart from search,
/// so search's cursor stays stable.
pub async fn featured(
    state: &RelayState,
    viewer: Option<Uuid>,
) -> Result<Vec<FeaturedAgent>, ApiError> {
    let ids: Vec<Uuid> = sqlx::query_scalar!(
        r#"
        SELECT a.id
          FROM agents a
          JOIN accounts acc ON acc.id = a.account_id
          LEFT JOIN agent_heartbeats h ON h.agent_id = a.id
         WHERE a.tombstoned_at IS NULL
           AND acc.tombstoned_at IS NULL
           AND a.visibility = 'network'
           AND (($1::text IS NOT NULL AND acc.slug = $1::text)
                OR (h.last_polled_at > now() - interval '5 minutes'
                    AND EXISTS (SELECT 1 FROM agent_capabilities c
                                 WHERE c.agent_id = a.id AND c.public_invoke = true)))
         ORDER BY ($1::text IS NOT NULL AND acc.slug = $1::text) DESC,
                  h.last_polled_at DESC NULLS LAST, a.created_at
         LIMIT $2
        "#,
        state.featured_account.as_deref(),
        FEATURED_LIMIT,
    )
    .fetch_all(&state.db)
    .await?;
    let loaded = fields::load(state, viewer, &ids).await?;
    let public = sqlx::query!(
        r#"
        SELECT id, agent_id, name, description
          FROM agent_capabilities
         WHERE agent_id = ANY($1) AND public_invoke = true AND visibility = 'network'
         ORDER BY name
        "#,
        &ids,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(loaded
        .into_iter()
        .map(|l| FeaturedAgent {
            public_capabilities: public
                .iter()
                .filter(|c| c.agent_id == l.summary.id)
                .map(|c| FeaturedCapability {
                    id: c.id,
                    name: c.name.clone(),
                    description: c.description.clone(),
                })
                .collect(),
            agent_slug: l.summary.slug.clone(),
            capabilities: l.capabilities,
            agent: l.summary,
        })
        .collect())
}
