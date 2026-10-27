//! Ranked agent search (spec §5.1). Shared by `GET /v1/discovery/agents`
//! and, from D3, the MCP `search_agents` tools.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::fields::{self, Access, AgentSummary, CapabilityBrief};
use super::standing::{self, You};
use crate::auth::AuthUser;
use crate::state::RelayState;

pub const DEFAULT_PAGE_SIZE: i64 = 20;
pub const MAX_PAGE_SIZE: i64 = 100;
/// Ranked results page by offset, up to this far.
pub const MAX_OFFSET: i64 = 1_000;
/// Cap free-text query length: a normal user types a handful of words.
pub const MAX_Q_LEN: usize = 500;
/// Cap `capability_schema` JSON length: parsing a 1 MB schema is wasted
/// work.
pub const MAX_CAP_SCHEMA_LEN: usize = 4096;
pub const MAX_CAP_NAME_LEN: usize = 128;

/// What to search for. Every filter is optional.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SearchParams {
    /// Free text: any word matches, and better matches rank higher.
    pub q: Option<String>,
    /// JSONB-encoded shape to match against any capability's output schema,
    /// e.g. `{"properties":{"slots":{}}}`.
    pub capability_schema: Option<String>,
    /// Part of a capability's name, case-insensitively.
    pub capability: Option<String>,
    /// `push` or `pull`.
    pub mode: Option<String>,
    pub verified: Option<bool>,
    /// Comma-separated tags. All must match.
    pub tags: Option<String>,
    /// Also list dormant agents: pull agents not seen in 30 days with
    /// nothing publicly callable.
    pub include_dormant: Option<bool>,
    /// Only agents you can call now: something public, or a grant to one of
    /// your agents in scope. Signed in only.
    pub callable: Option<bool>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// One search result.
#[derive(Debug, Clone, Serialize)]
pub struct SearchRow {
    #[serde(flatten)]
    pub agent: AgentSummary,
    pub capabilities: Vec<CapabilityBrief>,
    /// Deprecated, the same as `slug`. Kept for one release.
    pub agent_slug: String,
    /// Deprecated: see `capabilities[].access`. Kept for one release.
    pub has_public_capabilities: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub you: Option<You>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchPage {
    pub agents: Vec<SearchRow>,
    pub next_cursor: Option<String>,
    /// How many agents match. First page only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_estimate: Option<i64>,
}

/// A page position: ranked results page by offset, unranked ones by the
/// last row's `(created_at, id)`. Untagged, so cursors issued before ranking
/// (keyset) still decode.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Cursor {
    Offset { o: i64 },
    Keyset { created_at: DateTime<Utc>, id: Uuid },
}

fn encode_cursor(c: &Cursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(c).expect("cursor serializes"))
}

fn decode_cursor(s: &str) -> Option<Cursor> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(s).ok()?).ok()
}

fn invalid(msg: &str) -> ApiError {
    ApiError::InvalidRequest(msg.to_owned())
}

/// Whether `q` ranks: it has a word in it. `discovery_query` splits on the
/// same non-alphanumeric runs.
fn ranks(q: Option<&str>) -> bool {
    q.is_some_and(|q| q.chars().any(char::is_alphanumeric))
}

pub async fn search(
    state: &RelayState,
    viewer: Option<&AuthUser>,
    p: SearchParams,
) -> Result<SearchPage, ApiError> {
    let limit = p.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    if p.q.as_deref().map_or(0, str::len) > MAX_Q_LEN {
        return Err(invalid("q is too long"));
    }
    if p.capability_schema.as_deref().map_or(0, str::len) > MAX_CAP_SCHEMA_LEN {
        return Err(invalid("capability_schema is too long"));
    }
    if p.capability.as_deref().map_or(0, str::len) > MAX_CAP_NAME_LEN {
        return Err(invalid("capability is too long"));
    }
    let capability = p.capability.as_deref().filter(|s| !s.is_empty());
    let capability_schema = p
        .capability_schema
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(|_| invalid("capability_schema must be valid JSON"))?;
    let mode = match p.mode.as_deref() {
        None | Some("") => None,
        Some(m @ ("push" | "pull")) => Some(m.to_owned()),
        Some(_) => return Err(invalid("mode must be push|pull")),
    };
    let tags: Vec<String> = p
        .tags
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let callable = p.callable.unwrap_or(false);
    if callable && viewer.is_none() {
        return Err(invalid("callable=true needs a signed-in caller"));
    }

    let ranked = ranks(p.q.as_deref());
    let (offset, keyset) = match p.cursor.as_deref().map(decode_cursor) {
        None => (0, None),
        Some(None) => return Err(invalid("malformed cursor")),
        Some(Some(Cursor::Offset { o })) if ranked && (0..=MAX_OFFSET).contains(&o) => (o, None),
        Some(Some(Cursor::Keyset { created_at, id })) if !ranked => (0, Some((created_at, id))),
        Some(Some(_)) => return Err(invalid("this cursor is from a different search")),
    };

    // Your agents in scope: for `callable`, and later for standing.
    let mine = match viewer {
        Some(user) => standing::my_agents(state, user).await?,
        None => Vec::new(),
    };
    let my_ids: Vec<Uuid> = mine.iter().map(|a| a.id).collect();

    let rows = sqlx::query!(
        r#"
        WITH qry AS (SELECT discovery_query($1::text) AS tsq)
        SELECT a.id AS "id!", (count(*) OVER ())::bigint AS "total!"
          FROM agents a
          JOIN accounts acc ON acc.id = a.account_id
          LEFT JOIN agent_heartbeats h ON h.agent_id = a.id
         WHERE a.tombstoned_at IS NULL
           AND acc.tombstoned_at IS NULL
           -- Signed out: network-visible agents. Signed in: also your
           -- accounts' agents, and org agents of orgs you share.
           AND (a.visibility = 'network'
                OR ($11::uuid IS NOT NULL AND (
                    EXISTS (SELECT 1 FROM account_memberships m
                             WHERE m.account_id = a.account_id AND m.user_id = $11)
                    OR (a.visibility = 'org' AND EXISTS (
                        SELECT 1
                          FROM account_memberships caller_m
                          JOIN accounts shared_org ON shared_org.id = caller_m.account_id
                          JOIN account_memberships owner_m ON owner_m.account_id = shared_org.id
                          JOIN account_memberships owner_acc_m
                            ON owner_acc_m.user_id = owner_m.user_id
                           AND owner_acc_m.account_id = a.account_id
                         WHERE caller_m.user_id = $11
                           AND shared_org.account_type = 'organization')))))
           -- Dormant pull agents are hidden unless asked for. A new agent
           -- counts as seen when it's created.
           AND ($10::boolean OR NOT (
                   a.mode = 'pull'
                   AND COALESCE(h.last_polled_at, a.created_at) < now() - interval '30 days'
                   AND NOT EXISTS (
                       SELECT 1 FROM agent_capabilities c
                        WHERE c.agent_id = a.id AND c.public_invoke = true)))
           AND ((SELECT tsq FROM qry) IS NULL OR a.search_vec @@ (SELECT tsq FROM qry))
           AND ($2::jsonb IS NULL OR EXISTS (
                   SELECT 1 FROM agent_capabilities c
                    WHERE c.agent_id = a.id
                      AND c.visibility = 'network'
                      AND c.output_schema @> $2::jsonb))
           AND ($3::text IS NULL OR a.mode = $3::text)
           AND ($4::boolean IS NULL OR (acc.verified_at IS NOT NULL) = $4::boolean)
           AND ($5::text[] IS NULL OR a.tags @> $5::text[])
           -- A capability whose name contains the text: network-visible
           -- ones, or any of your own agents'.
           AND ($9::text IS NULL OR EXISTS (
                   SELECT 1 FROM agent_capabilities c
                    WHERE c.agent_id = a.id
                      AND strpos(lower(c.name), lower($9::text)) > 0
                      AND (c.visibility = 'network' OR ($11::uuid IS NOT NULL AND EXISTS (
                          SELECT 1 FROM account_memberships m
                           WHERE m.account_id = a.account_id AND m.user_id = $11)))))
           -- Callable now: something public, or a grant to one of yours.
           AND (NOT $13::boolean
                OR (a.visibility = 'network' AND EXISTS (
                       SELECT 1 FROM agent_capabilities c
                        WHERE c.agent_id = a.id AND c.public_invoke = true))
                OR EXISTS (
                       SELECT 1 FROM grants g
                        WHERE g.granter_agent_id = a.id
                          AND g.status = 'active'
                          AND (g.expires_at IS NULL OR g.expires_at > now())
                          AND g.grantee_agent_id = ANY($14::uuid[])))
           -- Unranked pages: rows strictly after the previous page's last.
           AND ($15::boolean OR $6::timestamptz IS NULL
                OR (a.created_at, a.id) < ($6::timestamptz, $7::uuid))
         ORDER BY
           -- 1. `q` is the agent's slug, display name or `account/slug`.
           (CASE WHEN $15::boolean
                 THEN lower(btrim($1::text)) IN (a.slug, lower(a.display_name), acc.slug || '/' || a.slug)
                 ELSE false END) DESC,
           -- 2. More words, in better places (default weights).
           (CASE WHEN $15::boolean
                 THEN ts_rank_cd(a.search_vec, (SELECT tsq FROM qry))
                 ELSE 0 END) DESC,
           -- 3. Seen in the last 5 minutes.
           (CASE WHEN $15::boolean
                 THEN COALESCE(h.last_polled_at > now() - interval '5 minutes', false)
                 ELSE false END) DESC,
           -- 4. Newest.
           a.created_at DESC, a.id DESC
         OFFSET $12
         LIMIT $8
        "#,
        p.q.as_deref(),
        capability_schema,
        mode,
        p.verified,
        (!tags.is_empty()).then_some(tags.as_slice()),
        keyset.map(|k| k.0),
        keyset.map(|k| k.1),
        limit + 1,
        capability,
        p.include_dormant.unwrap_or(false),
        viewer.map(|u| u.user_id),
        offset,
        callable,
        &my_ids,
        ranked,
    )
    .fetch_all(&state.db)
    .await?;

    let total = rows.first().map_or(0, |r| r.total);
    let has_more = rows.len() as i64 > limit;
    let ids: Vec<Uuid> = rows.iter().take(limit as usize).map(|r| r.id).collect();
    let loaded = fields::load(state, viewer.map(|u| u.user_id), &ids).await?;

    let next_cursor = if !has_more {
        None
    } else if ranked {
        let next = offset + limit;
        (next <= MAX_OFFSET).then(|| encode_cursor(&Cursor::Offset { o: next }))
    } else {
        loaded.last().map(|l| {
            encode_cursor(&Cursor::Keyset {
                created_at: l.summary.created_at,
                id: l.summary.id,
            })
        })
    };

    let mut standing = match viewer {
        Some(_) => standing::standing(state, &mine, &loaded).await?,
        None => Default::default(),
    };
    let agents = loaded
        .into_iter()
        .map(|l| SearchRow {
            you: viewer.map(|_| standing.remove(&l.summary.id).unwrap_or_default()),
            agent_slug: l.summary.slug.clone(),
            has_public_capabilities: l.capabilities.iter().any(|c| c.access == Access::Public),
            capabilities: l.capabilities,
            agent: l.summary,
        })
        .collect();

    Ok(SearchPage {
        agents,
        next_cursor,
        total_estimate: (p.cursor.is_none()).then_some(total),
    })
}
