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

#[derive(Debug, Clone, Serialize)]
pub struct FeaturedAgent {
    #[serde(flatten)]
    pub agent: AgentSummary,
    pub capabilities: Vec<CapabilityBrief>,
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
    Ok(loaded
        .into_iter()
        .map(|l| FeaturedAgent {
            capabilities: l.capabilities,
            agent: l.summary,
        })
        .collect())
}
