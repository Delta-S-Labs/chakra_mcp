//! A signed-in caller's standing with agents (spec §5.4): what they can
//! call now and as which agent, their friendships, and which of their
//! agents could start a call. Computed for the agents in the credential's
//! scope.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::fields::{Access, Loaded};
use crate::auth::AuthUser;
use crate::state::RelayState;

#[derive(Debug, Clone, Default, Serialize)]
pub struct You {
    pub callable: Vec<Callable>,
    pub friendships: Vec<Friendship>,
    /// Your agents that could start a call with this one.
    pub can_start_call_as: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Grant,
    Public,
}

#[derive(Debug, Clone, Serialize)]
pub struct Callable {
    pub capability: String,
    pub capability_id: Uuid,
    pub via: Via,
    /// The agent of yours holding the grant (the oldest grant's, when
    /// several do). `None` for a public capability: any of your agents can
    /// call it.
    pub as_agent_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Friendship {
    pub id: Uuid,
    pub your_agent_id: Uuid,
    /// `proposed` or `accepted`.
    pub status: String,
}

/// One of your agents in scope.
#[derive(Debug, Clone)]
pub struct Mine {
    pub id: Uuid,
    pub account_id: Uuid,
    pub mode: String,
}

/// Your agents the credential may act as: in accounts you belong to, not
/// deleted, within its scope (from the scope cache).
pub async fn my_agents(state: &RelayState, user: &AuthUser) -> Result<Vec<Mine>, ApiError> {
    let scope = crate::auth::agents_in_scope(state, user).await?;
    let rows = sqlx::query_as!(
        Mine,
        r#"
        SELECT a.id, a.account_id, a.mode
          FROM agents a
          JOIN account_memberships m ON m.account_id = a.account_id AND m.user_id = $1
         WHERE a.tombstoned_at IS NULL
         ORDER BY a.created_at, a.id
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|a| scope.as_ref().is_none_or(|set| set.contains(&a.id)))
        .collect())
}

/// Standing with each of `targets`, keyed by agent id. Two queries.
pub async fn standing(
    state: &RelayState,
    mine: &[Mine],
    targets: &[Loaded],
) -> Result<HashMap<Uuid, You>, ApiError> {
    let target_ids: Vec<Uuid> = targets.iter().map(|t| t.summary.id).collect();
    let my_ids: Vec<Uuid> = mine.iter().map(|a| a.id).collect();

    let grants = sqlx::query!(
        r#"
        SELECT g.granter_agent_id, g.grantee_agent_id, g.capability_id, c.name
          FROM grants g
          JOIN agent_capabilities c ON c.id = g.capability_id
         WHERE g.status = 'active'
           AND (g.expires_at IS NULL OR g.expires_at > now())
           AND g.granter_agent_id = ANY($1)
           AND g.grantee_agent_id = ANY($2)
         ORDER BY g.granted_at, g.id
        "#,
        &target_ids,
        &my_ids,
    )
    .fetch_all(&state.db)
    .await?;

    let friendships = sqlx::query!(
        r#"
        SELECT id, proposer_agent_id, target_agent_id, status
          FROM friendships
         WHERE status IN ('proposed', 'accepted')
           AND ((proposer_agent_id = ANY($1) AND target_agent_id = ANY($2))
             OR (target_agent_id = ANY($1) AND proposer_agent_id = ANY($2)))
         ORDER BY created_at, id
        "#,
        &my_ids,
        &target_ids,
    )
    .fetch_all(&state.db)
    .await?;

    let mut out = HashMap::new();
    for target in targets {
        let t = &target.summary;
        let mut you = You::default();

        let mut seen = HashSet::new();
        for g in grants.iter().filter(|g| g.granter_agent_id == t.id) {
            if g.grantee_agent_id != t.id && seen.insert(g.capability_id) {
                you.callable.push(Callable {
                    capability: g.name.clone(),
                    capability_id: g.capability_id,
                    via: Via::Grant,
                    as_agent_id: Some(g.grantee_agent_id),
                });
            }
        }
        for c in &target.capabilities {
            if c.access == Access::Public && seen.insert(c.id) {
                you.callable.push(Callable {
                    capability: c.name.clone(),
                    capability_id: c.id,
                    via: Via::Public,
                    as_agent_id: None,
                });
            }
        }

        let mut friends = HashSet::new();
        for f in &friendships {
            let yours = if f.target_agent_id == t.id {
                f.proposer_agent_id
            } else if f.proposer_agent_id == t.id {
                f.target_agent_id
            } else {
                continue;
            };
            if yours == t.id {
                continue;
            }
            if f.status == "accepted" {
                friends.insert(yours);
            }
            you.friendships.push(Friendship {
                id: f.id,
                your_agent_id: yours,
                status: f.status.clone(),
            });
        }

        // The rule `calls::lifecycle::start` applies: pull agents on both
        // sides; same account always, else `anyone`, else accepted friends.
        if state.calls.enabled && t.mode == "pull" && t.accept_calls != "nobody" {
            you.can_start_call_as = mine
                .iter()
                .filter(|a| a.id != t.id && a.mode == "pull")
                .filter(|a| {
                    a.account_id == target.account_id
                        || t.accept_calls == "anyone"
                        || friends.contains(&a.id)
                })
                .map(|a| a.id)
                .collect();
        }

        out.insert(t.id, you);
    }
    Ok(out)
}
