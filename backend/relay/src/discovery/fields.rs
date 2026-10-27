//! The fields every agent list shares (spec §5.2), built in one place.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::Relation;
use crate::state::RelayState;

/// An agent as discovery, profiles, the network list and the MCP tools show
/// it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSummary {
    pub id: Uuid,
    pub slug: String,
    pub account_slug: String,
    pub display_name: String,
    pub description: String,
    pub mode: String,
    pub tags: Vec<String>,
    pub visibility: String,
    /// Polled its inbox in the last 5 minutes. Push agents don't poll, so
    /// they never show as online.
    pub online: bool,
    /// Pull agents: the last inbox poll, else the creation time. Push
    /// agents: when their card was last fetched, else the creation time.
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub verified: bool,
    /// Belongs to the featured account (`DISCOVERY_FEATURED_ACCOUNT`).
    pub featured: bool,
    /// Accepted friendships, counted live.
    pub friend_count: i64,
    pub avg_rating: Option<f64>,
    pub review_count: i64,
    /// Who may start a call with it: `friends`, `anyone` or `nobody`.
    pub accept_calls: String,
}

/// Who can call a capability: `public` (anyone signed in, within its monthly
/// quota) or `friends` (needs a friendship and a grant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Public,
    Friends,
}

impl Access {
    /// A capability is public when its owner opted in (`public_invoke`, which
    /// requires a network-visible capability) and the agent is
    /// network-visible: the public invoke path refuses anything else.
    pub fn of(public_invoke: bool, agent_visibility: &str) -> Self {
        if public_invoke && agent_visibility == "network" {
            Self::Public
        } else {
            Self::Friends
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityBrief {
    pub id: Uuid,
    pub name: String,
    pub access: Access,
}

/// An agent loaded for a viewer: its shared fields, the capabilities the
/// viewer may see, and what the callers need to go further.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub summary: AgentSummary,
    pub capabilities: Vec<CapabilityBrief>,
    pub account_id: Uuid,
    pub account_display_name: String,
    pub relation: Relation,
}

/// An agent's effective call setting: push agents can't hold calls.
pub fn effective_accept_calls(mode: &str, stored: String) -> String {
    if mode == "push" {
        "nobody".into()
    } else {
        stored
    }
}

/// Load `ids` for `viewer` (a user id, or none when signed out), in `ids`'
/// order. Ids that don't exist or are deleted are skipped; visibility is the
/// caller's to check (with `Loaded::relation`). Two queries.
pub async fn load(
    state: &RelayState,
    viewer: Option<Uuid>,
    ids: &[Uuid],
) -> Result<Vec<Loaded>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query!(
        r#"
        SELECT a.id, a.slug, a.display_name, a.description, a.mode, a.tags, a.visibility,
               a.accept_calls, a.created_at, a.account_id,
               acc.slug AS account_slug, acc.display_name AS account_display_name,
               acc.verified_at IS NOT NULL AS "verified!",
               ($2::text IS NOT NULL AND acc.slug = $2::text) AS "featured!",
               COALESCE(h.last_polled_at > now() - interval '5 minutes', false) AS "online!",
               CASE WHEN a.mode = 'push' THEN COALESCE(a.agent_card_fetched_at, a.created_at)
                    ELSE COALESCE(h.last_polled_at, a.created_at) END AS "last_seen_at!",
               (SELECT count(*) FROM friendships f
                 WHERE f.status = 'accepted'
                   AND (f.proposer_agent_id = a.id OR f.target_agent_id = a.id)) AS "friend_count!",
               (SELECT AVG(rating)::float8 FROM agent_reviews r
                 WHERE r.target_agent_id = a.id AND r.hidden_at IS NULL) AS avg_rating,
               (SELECT count(*) FROM agent_reviews r
                 WHERE r.target_agent_id = a.id AND r.hidden_at IS NULL) AS "review_count!",
               ($3::uuid IS NOT NULL AND EXISTS (
                   SELECT 1 FROM account_memberships m
                    WHERE m.account_id = a.account_id AND m.user_id = $3)) AS "member!",
               ($3::uuid IS NOT NULL AND EXISTS (
                   SELECT 1
                     FROM account_memberships caller_m
                     JOIN accounts shared_org ON shared_org.id = caller_m.account_id
                     JOIN account_memberships owner_m ON owner_m.account_id = shared_org.id
                     JOIN account_memberships owner_acc_m
                       ON owner_acc_m.user_id = owner_m.user_id
                      AND owner_acc_m.account_id = a.account_id
                    WHERE caller_m.user_id = $3
                      AND shared_org.account_type = 'organization')) AS "shares_org!"
          FROM agents a
          JOIN accounts acc ON acc.id = a.account_id
          LEFT JOIN agent_heartbeats h ON h.agent_id = a.id
         WHERE a.id = ANY($1)
           AND a.tombstoned_at IS NULL
           AND acc.tombstoned_at IS NULL
        "#,
        ids,
        state.featured_account.as_deref(),
        viewer,
    )
    .fetch_all(&state.db)
    .await?;

    let caps = sqlx::query!(
        r#"
        SELECT id, agent_id, name, visibility, public_invoke
          FROM agent_capabilities
         WHERE agent_id = ANY($1)
         ORDER BY name
        "#,
        ids,
    )
    .fetch_all(&state.db)
    .await?;

    let mut loaded: Vec<Loaded> = rows
        .into_iter()
        .map(|r| {
            let relation = Relation {
                member: r.member,
                shares_org: r.shares_org,
            };
            let capabilities = caps
                .iter()
                .filter(|c| c.agent_id == r.id && relation.sees(&c.visibility))
                .map(|c| CapabilityBrief {
                    id: c.id,
                    name: c.name.clone(),
                    access: Access::of(c.public_invoke, &r.visibility),
                })
                .collect();
            Loaded {
                summary: AgentSummary {
                    id: r.id,
                    slug: r.slug,
                    account_slug: r.account_slug,
                    display_name: r.display_name,
                    description: r.description,
                    accept_calls: effective_accept_calls(&r.mode, r.accept_calls),
                    mode: r.mode,
                    tags: r.tags,
                    visibility: r.visibility,
                    online: r.online,
                    last_seen_at: r.last_seen_at,
                    created_at: r.created_at,
                    verified: r.verified,
                    featured: r.featured,
                    friend_count: r.friend_count,
                    avg_rating: r.avg_rating,
                    review_count: r.review_count,
                },
                capabilities,
                account_id: r.account_id,
                account_display_name: r.account_display_name,
                relation,
            }
        })
        .collect();
    loaded.sort_by_key(|l| ids.iter().position(|id| *id == l.summary.id));
    Ok(loaded)
}
