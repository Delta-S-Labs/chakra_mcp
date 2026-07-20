//! Usage roll-ups across the caller's account.
//!
//! Two endpoints live here:
//!
//!   * `GET /v1/usage/summary?from=&to=`
//!     — every-direction roll-up of the caller's relay traffic.
//!     The /app/usage page hits this exactly once and slices the
//!     result into total + by_org / by_agent / by_api_key / by_pair.
//!
//!   * `GET /v1/pairings/{kind}/{id}/usage?from=&to=`
//!     — per-pair traffic, mirroring the shape of the existing
//!     `/v1/api-keys/{id}/usage` endpoint. Pairs are filtered by
//!     joining `relay_invocations.minted_jti` against the
//!     device-flow / oauth-code row's `minted_jti`.
//!
//! Scope: the summary endpoint counts every row where the user is on
//! either side of the call — caller (via api_key.user_id or
//! JWT.user_id stamped on the row's invoked_by_user_id) OR target
//! (via target_agent owned by one of the user's accounts). The pair
//! endpoint adds a `minted_jti = ...` filter on top of that scope.
//!
//! All date ranges default to `now - 30 days .. now` and reject
//! inverted ranges with 400. Top-N truncation matches the existing
//! per-key handler (5 capabilities, 20 agents).

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::state::AppState;

// ─────────────────────────────────────────────────────────
// Shared query + response shapes
// ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    /// ISO-8601 timestamp. Defaults to `now - 30 days`.
    pub from: Option<DateTime<Utc>>,
    /// ISO-8601 timestamp. Defaults to `now`.
    pub to: Option<DateTime<Utc>>,
    /// Org-wide (every member's activity) or personal (caller only).
    /// Only affects the `by_action` breakdown — the other rollups stay
    /// membership-scoped. Default `org`, matching the existing
    /// dimension-rollup behaviour. See the 2026-05-16 design doc.
    #[serde(default)]
    pub scope: ActionScope,
}

/// Personal vs org scope for the `by_action` breakdown.
///
/// `Org` (default): every count joins through `account_memberships` to
/// the caller's accounts. Reads from pre-attribution rows too.
///
/// `Personal`: same membership join PLUS the relevant `*_user_id`
/// column equals the caller. Pre-migration rows are NULL and silently
/// excluded — Decision 4 in the spec covers the user-visible footnote.
#[derive(Debug, Deserialize, Serialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionScope {
    #[default]
    Org,
    Personal,
}

fn resolve_range(q: &RangeQuery) -> Result<(DateTime<Utc>, DateTime<Utc>), ApiError> {
    let to = q.to.unwrap_or_else(Utc::now);
    let from = q.from.unwrap_or_else(|| to - Duration::days(30));
    if from >= to {
        return Err(ApiError::InvalidRequest(
            "`from` must be strictly before `to`".into(),
        ));
    }
    Ok((from, to))
}

#[derive(Debug, Serialize)]
pub struct DailyCount {
    pub date: chrono::NaiveDate,
    pub requests: i64,
}

// ─────────────────────────────────────────────────────────
// GET /v1/usage/summary
// ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TotalCount {
    pub requests: i64,
    pub succeeded: i64,
    pub failed: i64,
}

#[derive(Debug, Serialize)]
pub struct OrgRollup {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct AgentRollup {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyRollup {
    pub id: Uuid,
    pub name: String,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct PairRollup {
    /// "device_flow" or "oauth" — matches the `kind` axis on
    /// `/v1/pairings`. There's no "api_key" entry here because API
    /// keys already have their own roll-up (`by_api_key`).
    pub kind: String,
    pub id: Uuid,
    pub label: String,
    pub requests: i64,
}

#[derive(Debug, Serialize)]
pub struct CapabilityRollup {
    /// Snapshot of the capability name at invocation time
    /// (`relay_invocations.capability_name`). Capabilities can be
    /// renamed; the audit log keeps the historical name.
    pub name: String,
    pub requests: i64,
}

/// Counts of platform-level actions in the window — non-invocation
/// activity that the existing four rollups miss entirely (friendship
/// proposals, grants issued, capabilities published, etc.). The
/// `scope` echoes the query param so the UI can confirm what it got.
#[derive(Debug, Serialize)]
pub struct ActionBreakdown {
    pub scope: ActionScope,
    pub inbox_invocations: i64,
    pub friendships_proposed: i64,
    pub friendships_accepted: i64,
    pub friendships_rejected: i64,
    pub friendships_cancelled: i64,
    pub grants_issued: i64,
    pub grants_revoked: i64,
    pub agents_registered: i64,
    pub capabilities_published: i64,
}

#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub total: TotalCount,
    pub by_org: Vec<OrgRollup>,
    pub by_agent: Vec<AgentRollup>,
    pub by_api_key: Vec<ApiKeyRollup>,
    pub by_pair: Vec<PairRollup>,
    pub by_capability: Vec<CapabilityRollup>,
    pub by_action: ActionBreakdown,
    pub daily: Vec<DailyCount>,
}

/// Roll up every relay invocation the caller can see.
///
/// Scope clause is reused across each sub-query — we keep it inline as a
/// CTE per query rather than materialising a temp table because the
/// query planner inlines the CTE into each shape and the cost across
/// six aggregates is the same as one. Worth the small repetition.
pub async fn summary(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<SummaryResponse>> {
    let (from, to) = resolve_range(&q)?;

    // ── Total ───────────────────────────────────────────
    let totals = sqlx::query!(
        r#"
        SELECT
            COUNT(*) AS "requests!",
            COUNT(*) FILTER (WHERE i.status = 'succeeded') AS "succeeded!",
            COUNT(*) FILTER (WHERE i.status IN ('failed', 'timeout', 'rejected')) AS "failed!"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.created_at >= $2
          AND i.created_at <  $3
          AND (
              i.invoked_by_user_id = $1
           OR ea.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ga.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_one(&state.db)
    .await?;

    // ── By org (caller's accounts only) ─────────────────
    // We attribute each row to the *grantee*'s account — that's the
    // caller-side credit. Orgs the user is a member of show up; the
    // user's personal account shows up too if their own caller_agent
    // sourced the call.
    let by_org_rows = sqlx::query!(
        r#"
        SELECT
            a.id     AS "id!",
            a.slug   AS "slug!",
            a.display_name AS "display_name!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        JOIN agents ea ON ea.id = i.grantee_agent_id
        JOIN accounts a ON a.id = ea.account_id
        WHERE i.created_at >= $2
          AND i.created_at <  $3
          AND ea.account_id IN (
              SELECT account_id FROM account_memberships WHERE user_id = $1
          )
        GROUP BY a.id, a.slug, a.display_name
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_org: Vec<OrgRollup> = by_org_rows
        .into_iter()
        .map(|r| OrgRollup {
            id: r.id,
            slug: r.slug,
            display_name: r.display_name,
            requests: r.requests,
        })
        .collect();

    // ── By caller agent ─────────────────────────────────
    // The agent that ran AS the caller (grantee in the invocation row).
    let by_agent_rows = sqlx::query!(
        r#"
        SELECT
            ea.id   AS "id!",
            ea.slug AS "slug!",
            ea.display_name AS "name!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.created_at >= $2
          AND i.created_at <  $3
          AND ea.account_id IN (
              SELECT account_id FROM account_memberships WHERE user_id = $1
          )
        GROUP BY ea.id, ea.slug, ea.display_name
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_agent: Vec<AgentRollup> = by_agent_rows
        .into_iter()
        .map(|r| AgentRollup {
            id: r.id,
            slug: r.slug,
            name: r.name,
            requests: r.requests,
        })
        .collect();

    // ── By API key (caller's keys only) ─────────────────
    let by_key_rows = sqlx::query!(
        r#"
        SELECT
            k.id     AS "id!",
            k.name   AS "name!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        JOIN api_keys k ON k.id = i.api_key_id
        WHERE k.user_id = $1
          AND i.created_at >= $2
          AND i.created_at <  $3
        GROUP BY k.id, k.name
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_api_key: Vec<ApiKeyRollup> = by_key_rows
        .into_iter()
        .map(|r| ApiKeyRollup {
            id: r.id,
            name: r.name,
            requests: r.requests,
        })
        .collect();

    // ── By paired session (device_flow + oauth) ─────────
    // UNION ALL across the two minted_jti sources; each row joins the
    // invocation count from `relay_invocations.minted_jti` against the
    // pairing's own `minted_jti`. Label resolution mirrors
    // `handlers/pairings.rs::list` so the same string shows up in
    // both lists.
    let by_pair_device_rows = sqlx::query!(
        r#"
        SELECT
            d.id       AS "id!",
            COALESCE(a.display_name, a.slug, '(pair)') AS "label!",
            COUNT(*)   AS "requests!"
        FROM relay_invocations i
        JOIN oauth_device_codes d ON d.minted_jti = i.minted_jti
        LEFT JOIN agents a ON a.id = d.approved_agent_id
        WHERE d.approved_user_id = $1
          AND i.created_at >= $2
          AND i.created_at <  $3
        GROUP BY d.id, a.display_name, a.slug
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_pair_oauth_rows = sqlx::query!(
        r#"
        SELECT
            o.id     AS "id!",
            c.client_name AS "label!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        JOIN oauth_authorizations o ON o.minted_jti = i.minted_jti
        JOIN oauth_clients c ON c.client_id = o.client_id
        WHERE o.user_id = $1
          AND i.created_at >= $2
          AND i.created_at <  $3
        GROUP BY o.id, c.client_name
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;

    let mut by_pair: Vec<PairRollup> =
        Vec::with_capacity(by_pair_device_rows.len() + by_pair_oauth_rows.len());
    for r in by_pair_device_rows {
        by_pair.push(PairRollup {
            kind: "device_flow".into(),
            id: r.id,
            label: r.label,
            requests: r.requests,
        });
    }
    for r in by_pair_oauth_rows {
        by_pair.push(PairRollup {
            kind: "oauth".into(),
            id: r.id,
            label: r.label,
            requests: r.requests,
        });
    }
    by_pair.sort_by_key(|p| std::cmp::Reverse(p.requests));
    by_pair.truncate(20);

    // ── By capability ───────────────────────────────────
    // Same caller-scope clause as the total query (caller-side OR
    // either-account-side). Top 20 capabilities by count, matching the
    // by_agent / by_api_key convention.
    let by_capability_rows = sqlx::query!(
        r#"
        SELECT
            i.capability_name AS "name!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.created_at >= $2
          AND i.created_at <  $3
          AND (
              i.invoked_by_user_id = $1
           OR ea.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ga.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
        GROUP BY i.capability_name
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_capability: Vec<CapabilityRollup> = by_capability_rows
        .into_iter()
        .map(|r| CapabilityRollup {
            name: r.name,
            requests: r.requests,
        })
        .collect();

    // ── By platform action ─────────────────────────────
    // Nine fixed counts. Org scope joins through account_memberships
    // and ignores the *_user_id attribution columns; Personal scope
    // additionally pins each row to the caller via the table's actor
    // column. The Personal branch returns 0 for pre-migration rows
    // where the actor column is NULL.
    let by_action = compute_by_action(&state.db, user.user_id, from, to, q.scope).await?;

    // ── Daily sparkline ─────────────────────────────────
    let daily_rows = sqlx::query!(
        r#"
        SELECT
            date_trunc('day', i.created_at)::date AS "date!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.created_at >= $2
          AND i.created_at <  $3
          AND (
              i.invoked_by_user_id = $1
           OR ea.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ga.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
        GROUP BY 1
        ORDER BY 1 ASC
        "#,
        user.user_id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let daily: Vec<DailyCount> = daily_rows
        .into_iter()
        .map(|r| DailyCount {
            date: r.date,
            requests: r.requests,
        })
        .collect();

    Ok(Json(SummaryResponse {
        from,
        to,
        total: TotalCount {
            requests: totals.requests,
            succeeded: totals.succeeded,
            failed: totals.failed,
        },
        by_org,
        by_agent,
        by_api_key,
        by_pair,
        by_capability,
        by_action,
        daily,
    }))
}

/// Fetch the nine `by_action` counts for the caller in the given
/// window. The Personal branch reads the `*_user_id` attribution
/// columns added in migration 0019 (plus the pre-existing columns
/// from migrations 0003/0005); the Org branch ignores those columns
/// entirely and uses membership scoping only.
async fn compute_by_action(
    db: &sqlx::PgPool,
    caller: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    scope: ActionScope,
) -> Result<ActionBreakdown, ApiError> {
    let personal = matches!(scope, ActionScope::Personal);

    // ── Inbox invocations ───────────────────────────────
    // Membership-scoped on either side of the invocation. Personal
    // additionally constrains to invoked_by_user_id = caller.
    let inbox_invocations = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM relay_invocations i
        LEFT JOIN agents ga ON ga.id = i.granter_agent_id
        LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.created_at >= $2
          AND i.created_at <  $3
          AND (
              i.invoked_by_user_id = $1
           OR ea.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ga.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
          AND ( NOT $4::bool OR i.invoked_by_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
    )
    .fetch_one(db)
    .await?;

    // ── Friendships proposed ────────────────────────────
    // A friendship row touches the caller's org if either side's
    // agent lives in one of their accounts. Personal: proposer == me.
    let friendships_proposed = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM friendships f
        JOIN agents pa ON pa.id = f.proposer_agent_id
        JOIN agents ta ON ta.id = f.target_agent_id
        WHERE f.created_at >= $2
          AND f.created_at <  $3
          AND (
              pa.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ta.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
          AND ( NOT $4::bool OR f.proposer_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
    )
    .fetch_one(db)
    .await?;

    // ── Friendships accepted / rejected / cancelled ─────
    // Decision timestamp is `decided_at`; status filter selects the
    // transition. Personal: decided_by_user_id = me.
    let friendships_accepted =
        friendships_transition_count(db, caller, from, to, personal, "accepted").await?;
    let friendships_rejected =
        friendships_transition_count(db, caller, from, to, personal, "rejected").await?;
    let friendships_cancelled =
        friendships_transition_count(db, caller, from, to, personal, "cancelled").await?;

    // ── Grants issued ───────────────────────────────────
    // Membership-scoped via either side's account; Personal:
    // granted_by_user_id = me.
    let grants_issued = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        WHERE g.created_at >= $2
          AND g.created_at <  $3
          AND (
              ga.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ea.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
          AND ( NOT $4::bool OR g.granted_by_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
    )
    .fetch_one(db)
    .await?;

    // ── Grants revoked ──────────────────────────────────
    // Same membership scope, but bucket by `revoked_at` (so the count
    // lands in the window the revocation happened in, not the
    // window the grant was first issued). Personal:
    // revoked_by_user_id = me.
    let grants_revoked = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM grants g
        JOIN agents ga ON ga.id = g.granter_agent_id
        JOIN agents ea ON ea.id = g.grantee_agent_id
        WHERE g.revoked_at IS NOT NULL
          AND g.revoked_at >= $2
          AND g.revoked_at <  $3
          AND (
              ga.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ea.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
          AND ( NOT $4::bool OR g.revoked_by_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
    )
    .fetch_one(db)
    .await?;

    // ── Agents registered ───────────────────────────────
    // Org: every agent in an account the caller belongs to.
    // Personal: created_by_user_id = me.
    let agents_registered = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM agents a
        WHERE a.created_at >= $2
          AND a.created_at <  $3
          AND a.account_id IN (
              SELECT account_id FROM account_memberships WHERE user_id = $1
          )
          AND ( NOT $4::bool OR a.created_by_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
    )
    .fetch_one(db)
    .await?;

    // ── Capabilities published ──────────────────────────
    // Each capability lives under an agent; scope to the caller's
    // accounts via that agent. Personal: created_by_user_id = me.
    let capabilities_published = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM agent_capabilities c
        JOIN agents a ON a.id = c.agent_id
        WHERE c.created_at >= $2
          AND c.created_at <  $3
          AND a.account_id IN (
              SELECT account_id FROM account_memberships WHERE user_id = $1
          )
          AND ( NOT $4::bool OR c.created_by_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
    )
    .fetch_one(db)
    .await?;

    Ok(ActionBreakdown {
        scope,
        inbox_invocations,
        friendships_proposed,
        friendships_accepted,
        friendships_rejected,
        friendships_cancelled,
        grants_issued,
        grants_revoked,
        agents_registered,
        capabilities_published,
    })
}

/// Count of `friendships` rows in the window whose current `status`
/// matches the supplied transition. The transition is timestamped via
/// `decided_at`; the membership scope mirrors `friendships_proposed`.
async fn friendships_transition_count(
    db: &sqlx::PgPool,
    caller: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    personal: bool,
    status: &str,
) -> Result<i64, ApiError> {
    let c = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "c!"
        FROM friendships f
        JOIN agents pa ON pa.id = f.proposer_agent_id
        JOIN agents ta ON ta.id = f.target_agent_id
        WHERE f.decided_at IS NOT NULL
          AND f.decided_at >= $2
          AND f.decided_at <  $3
          AND f.status = $5
          AND (
              pa.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
           OR ta.account_id IN (
                  SELECT account_id FROM account_memberships WHERE user_id = $1
              )
          )
          AND ( NOT $4::bool OR f.decided_by_user_id = $1 )
        "#,
        caller,
        from,
        to,
        personal,
        status,
    )
    .fetch_one(db)
    .await?;
    Ok(c)
}

// ─────────────────────────────────────────────────────────
// GET /v1/pairings/{kind}/{id}/usage
// ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PairingUsageResponse {
    pub kind: String,
    pub id: Uuid,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub total_requests: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub by_agent: Vec<AgentRollup>,
    pub daily: Vec<DailyCount>,
}

/// Per-pair usage. Filter shape mirrors the per-api-key endpoint, but
/// joins on `minted_jti` instead of `api_key_id`. Callers must own the
/// pair — we look up `approved_user_id` / `user_id` against the
/// caller's id.
pub async fn pairing_usage(
    State(state): State<AppState>,
    user: AuthUser,
    Path((kind, id)): Path<(String, Uuid)>,
    Query(q): Query<RangeQuery>,
) -> ApiResult<Json<PairingUsageResponse>> {
    let (from, to) = resolve_range(&q)?;

    // Resolve the pair's minted_jti AND check ownership at the same
    // time. Treat "not found" and "not yours" identically (404) so we
    // don't leak existence of pairs the caller doesn't own.
    let minted_jti: Option<Uuid> = match kind.as_str() {
        "device_flow" => {
            let row = sqlx::query!(
                r#"SELECT approved_user_id, minted_jti FROM oauth_device_codes WHERE id = $1"#,
                id,
            )
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
            if row.approved_user_id != Some(user.user_id) {
                return Err(ApiError::NotFound);
            }
            row.minted_jti
        }
        "oauth" => {
            let row = sqlx::query!(
                r#"SELECT user_id, minted_jti FROM oauth_authorizations WHERE id = $1"#,
                id,
            )
            .fetch_optional(&state.db)
            .await?
            .ok_or(ApiError::NotFound)?;
            if row.user_id != user.user_id {
                return Err(ApiError::NotFound);
            }
            row.minted_jti
        }
        // api_key pairings have their own dedicated usage endpoint;
        // routing here would just shadow that. Reject with 400.
        "api_key" => {
            return Err(ApiError::InvalidRequest(
                "use /v1/api-keys/{id}/usage for api_key pairings".into(),
            ));
        }
        _ => {
            return Err(ApiError::InvalidRequest(format!(
                "unknown pairing kind '{kind}'; expected one of device_flow, oauth"
            )));
        }
    };

    // If the pair never minted a token (or did so before migration
    // 0017 stamped `minted_jti`), there's nothing to roll up. Short-
    // circuit with an empty response in the right shape.
    let Some(jti) = minted_jti else {
        return Ok(Json(PairingUsageResponse {
            kind,
            id,
            from,
            to,
            total_requests: 0,
            succeeded: 0,
            failed: 0,
            by_agent: vec![],
            daily: vec![],
        }));
    };

    let totals = sqlx::query!(
        r#"
        SELECT
            COUNT(*) AS "total!",
            COUNT(*) FILTER (WHERE status = 'succeeded') AS "succeeded!",
            COUNT(*) FILTER (WHERE status IN ('failed', 'timeout', 'rejected')) AS "failed!"
        FROM relay_invocations
        WHERE minted_jti = $1
          AND created_at >= $2
          AND created_at <  $3
        "#,
        jti,
        from,
        to,
    )
    .fetch_one(&state.db)
    .await?;

    let by_agent_rows = sqlx::query!(
        r#"
        SELECT
            ea.id   AS "id!",
            ea.slug AS "slug!",
            ea.display_name AS "name!",
            COUNT(*) AS "requests!"
        FROM relay_invocations i
        JOIN agents ea ON ea.id = i.grantee_agent_id
        WHERE i.minted_jti = $1
          AND i.created_at >= $2
          AND i.created_at <  $3
        GROUP BY ea.id, ea.slug, ea.display_name
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        jti,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_agent: Vec<AgentRollup> = by_agent_rows
        .into_iter()
        .map(|r| AgentRollup {
            id: r.id,
            slug: r.slug,
            name: r.name,
            requests: r.requests,
        })
        .collect();

    let daily_rows = sqlx::query!(
        r#"
        SELECT
            date_trunc('day', created_at)::date AS "date!",
            COUNT(*) AS "requests!"
        FROM relay_invocations
        WHERE minted_jti = $1
          AND created_at >= $2
          AND created_at <  $3
        GROUP BY 1
        ORDER BY 1 ASC
        "#,
        jti,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let daily: Vec<DailyCount> = daily_rows
        .into_iter()
        .map(|r| DailyCount {
            date: r.date,
            requests: r.requests,
        })
        .collect();

    Ok(Json(PairingUsageResponse {
        kind,
        id,
        from,
        to,
        total_requests: totals.total,
        succeeded: totals.succeeded,
        failed: totals.failed,
        by_agent,
        daily,
    }))
}

#[cfg(test)]
mod tests {
    //! Coverage:
    //!   * /v1/usage/summary returns zeros for a fresh user
    //!   * /v1/usage/summary picks up an api_key-attributed invocation
    //!   * /v1/pairings/device_flow/{id}/usage returns the pair's traffic
    //!   * /v1/pairings/device_flow/{id}/usage is 404 for someone else's pair

    use crate::tests_support::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Insert a minimal invocation row for `caller_user`, attributed to
    /// `caller_agent` (grantee). `minted_jti` and `api_key_id` are
    /// optional so each test can pick the right pathway.
    #[allow(clippy::too_many_arguments)]
    async fn seed_invocation(
        pool: &PgPool,
        caller_user: Uuid,
        granter_agent: Uuid,
        caller_agent: Uuid,
        capability: Uuid,
        api_key_id: Option<Uuid>,
        minted_jti: Option<Uuid>,
        status: &str,
    ) {
        seed_invocation_named(
            pool,
            caller_user,
            granter_agent,
            caller_agent,
            capability,
            "do",
            api_key_id,
            minted_jti,
            status,
        )
        .await;
    }

    /// Same as `seed_invocation` but lets the test pick the
    /// `capability_name` snapshot. Used by the by_capability test
    /// (which needs distinct names across rows) — the underlying
    /// `agent_capabilities.name` is unchanged so the unique constraint
    /// stays happy.
    #[allow(clippy::too_many_arguments)]
    async fn seed_invocation_named(
        pool: &PgPool,
        caller_user: Uuid,
        granter_agent: Uuid,
        caller_agent: Uuid,
        capability: Uuid,
        capability_name: &str,
        api_key_id: Option<Uuid>,
        minted_jti: Option<Uuid>,
        status: &str,
    ) {
        // Stamp created_at from the *process* clock rather than letting it
        // default to the database's now().
        //
        // `resolve_range` bounds every usage query with `to = Utc::now()`
        // taken in-process, so a row written on the DB clock is only counted
        // if the two clocks agree. They don't always: with Postgres in a
        // Docker Desktop VM the container clock can sit tens of milliseconds
        // *ahead* of the macOS host, which puts every freshly-seeded row in
        // the future relative to `to` and silently filters it out — the whole
        // suite then reports 0 for counts it just inserted. CI never sees it
        // (same kernel, one clock), so it looks like local-only flake.
        //
        // Backdating by a second keeps both bounds on one clock and leaves
        // the row comfortably inside the default 30-day window.
        let created_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        sqlx::query!(
            r#"
            INSERT INTO relay_invocations
                (id, granter_agent_id, grantee_agent_id, capability_id,
                 capability_name, invoked_by_user_id, status, elapsed_ms,
                 api_key_id, minted_jti, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, $9, $10)
            "#,
            Uuid::now_v7(),
            granter_agent,
            caller_agent,
            capability,
            capability_name,
            caller_user,
            status,
            api_key_id,
            minted_jti,
            created_at,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_capability(pool: &PgPool, agent: Uuid) -> Uuid {
        let cap = Uuid::now_v7();
        // Process-clock created_at — see seed_invocation_named.
        let created_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                 (id, agent_id, name, description, input_schema, output_schema,
                  created_at)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb, $3)"#,
            cap,
            agent,
            created_at,
        )
        .execute(pool)
        .await
        .unwrap();
        cap
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn summary_returns_zeros_for_fresh_user(pool: PgPool) {
        let (_user, jwt, _personal) = seed_user_with_personal(&pool, "alice").await;
        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/usage/summary")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"]["requests"], 0);
        assert_eq!(json["by_agent"].as_array().unwrap().len(), 0);
        assert_eq!(json["by_api_key"].as_array().unwrap().len(), 0);
        assert_eq!(json["by_pair"].as_array().unwrap().len(), 0);
        assert_eq!(json["daily"].as_array().unwrap().len(), 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn summary_counts_api_key_invocations(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let caller_agent = seed_agent(&pool, personal, "hermes", user.user_id).await;
        let granter_agent = seed_agent(&pool, personal, "skytasker", user.user_id).await;
        let cap = seed_capability(&pool, granter_agent).await;
        let (key_id, _) = seed_api_key(&pool, user.user_id, "ci key").await;

        for _ in 0..3 {
            seed_invocation(
                &pool,
                user.user_id,
                granter_agent,
                caller_agent,
                cap,
                Some(key_id),
                None,
                "succeeded",
            )
            .await;
        }
        seed_invocation(
            &pool,
            user.user_id,
            granter_agent,
            caller_agent,
            cap,
            Some(key_id),
            None,
            "failed",
        )
        .await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/usage/summary")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"]["requests"], 4);
        assert_eq!(json["total"]["succeeded"], 3);
        assert_eq!(json["total"]["failed"], 1);
        let by_key = json["by_api_key"].as_array().unwrap();
        assert_eq!(by_key.len(), 1);
        assert_eq!(by_key[0]["id"], serde_json::json!(key_id));
        assert_eq!(by_key[0]["requests"], 4);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn pairing_usage_counts_jwt_invocations(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let caller_agent = seed_agent(&pool, personal, "hermes", user.user_id).await;
        let granter_agent = seed_agent(&pool, personal, "skytasker", user.user_id).await;
        let cap = seed_capability(&pool, granter_agent).await;
        let device_id = seed_approved_device_flow(&pool, user.user_id, caller_agent).await;

        // Stamp a minted_jti so the join below has something to match.
        let pair_jti = Uuid::now_v7();
        sqlx::query!(
            "UPDATE oauth_device_codes SET minted_jti = $1 WHERE id = $2",
            pair_jti,
            device_id,
        )
        .execute(&pool)
        .await
        .unwrap();

        for _ in 0..2 {
            seed_invocation(
                &pool,
                user.user_id,
                granter_agent,
                caller_agent,
                cap,
                None,
                Some(pair_jti),
                "succeeded",
            )
            .await;
        }

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/pairings/device_flow/{device_id}/usage"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total_requests"], 2);
        assert_eq!(json["succeeded"], 2);
    }

    // ─── PR 3 additions ───────────────────────────────
    //
    // Coverage:
    //   * by_capability groups + sorts by snapshot name
    //   * by_action Personal scope returns only caller-attributed rows
    //   * by_action Org scope returns every member's rows in shared
    //     accounts
    //   * friendships transition counts respond to decided_by_user_id /
    //     status

    /// Add `user_id` to `account_id` as a `member`-role membership so a
    /// second user shares the same Org scope as the first.
    async fn add_member(pool: &PgPool, account_id: Uuid, user_id: Uuid) {
        sqlx::query!(
            r#"
            INSERT INTO account_memberships (id, account_id, user_id, role)
            VALUES ($1, $2, $3, 'member')
            "#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a friendships row directly so a test can pick the
    /// proposer / decider / status / decided_at fields. Returns the
    /// row id.
    #[allow(clippy::too_many_arguments)]
    async fn seed_friendship(
        pool: &PgPool,
        proposer_agent: Uuid,
        target_agent: Uuid,
        proposer_user: Option<Uuid>,
        decided_by_user: Option<Uuid>,
        status: &str,
        decided: bool,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let decided_at: Option<chrono::DateTime<chrono::Utc>> = if decided {
            Some(chrono::Utc::now())
        } else {
            None
        };
        // Process-clock created_at — see seed_invocation_named for why the
        // database default can't be trusted here (by_action filters on
        // f.created_at).
        let created_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        sqlx::query!(
            r#"
            INSERT INTO friendships
                (id, proposer_agent_id, target_agent_id, status,
                 proposer_user_id, decided_by_user_id, decided_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
            id,
            proposer_agent,
            target_agent,
            status,
            proposer_user,
            decided_by_user,
            decided_at,
            created_at,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Insert a grants row directly with full attribution control.
    #[allow(clippy::too_many_arguments)]
    async fn seed_grant(
        pool: &PgPool,
        granter_agent: Uuid,
        grantee_agent: Uuid,
        capability: Uuid,
        granted_by: Option<Uuid>,
        revoked_by: Option<Uuid>,
        revoked: bool,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let (status, revoked_at) = if revoked {
            ("revoked", Some(chrono::Utc::now()))
        } else {
            ("active", None)
        };
        // Process-clock created_at — see seed_invocation_named for why the
        // database default can't be trusted here (by_action filters on
        // g.created_at).
        let created_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        sqlx::query!(
            r#"
            INSERT INTO grants
                (id, granter_agent_id, grantee_agent_id, capability_id,
                 status, granted_by_user_id, revoked_by_user_id, revoked_at,
                 created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            id,
            granter_agent,
            grantee_agent,
            capability,
            status,
            granted_by,
            revoked_by,
            revoked_at,
            created_at,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Insert an `agent_capabilities` row with a custom name and
    /// `created_by_user_id`. The seed_capability helper above always
    /// inserts as 'do' and leaves attribution NULL.
    async fn seed_capability_attributed(
        pool: &PgPool,
        agent: Uuid,
        name: &str,
        created_by: Option<Uuid>,
    ) -> Uuid {
        let cap = Uuid::now_v7();
        // Process-clock created_at — see seed_invocation_named (by_action
        // counts capabilities_published off c.created_at).
        let created_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                 (id, agent_id, name, description, input_schema, output_schema,
                  created_by_user_id, created_at)
               VALUES ($1, $2, $3, 'Do.', '{}'::jsonb, '{}'::jsonb, $4, $5)"#,
            cap,
            agent,
            name,
            created_by,
            created_at,
        )
        .execute(pool)
        .await
        .unwrap();
        cap
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn summary_by_capability_aggregates_by_name(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let caller_agent = seed_agent(&pool, personal, "hermes", user.user_id).await;
        let granter_agent = seed_agent(&pool, personal, "skytasker", user.user_id).await;
        let cap = seed_capability(&pool, granter_agent).await;

        // Two `message_owner` + one `schedule_meeting` — the rollup
        // groups by snapshot name, so order must be by count desc.
        for _ in 0..2 {
            seed_invocation_named(
                &pool,
                user.user_id,
                granter_agent,
                caller_agent,
                cap,
                "message_owner",
                None,
                None,
                "succeeded",
            )
            .await;
        }
        seed_invocation_named(
            &pool,
            user.user_id,
            granter_agent,
            caller_agent,
            cap,
            "schedule_meeting",
            None,
            None,
            "succeeded",
        )
        .await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/usage/summary")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let by_cap = json["by_capability"].as_array().unwrap();
        assert_eq!(by_cap.len(), 2);
        assert_eq!(by_cap[0]["name"], "message_owner");
        assert_eq!(by_cap[0]["requests"], 2);
        assert_eq!(by_cap[1]["name"], "schedule_meeting");
        assert_eq!(by_cap[1]["requests"], 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn summary_by_action_personal_excludes_teammate_rows(pool: PgPool) {
        // Alice + Bob share `org`. Both perform a friendship
        // proposal + a capability publish; Alice asks for Personal
        // scope and must only see her own work.
        let (alice, alice_jwt, _alice_personal) = seed_user_with_personal(&pool, "alice").await;
        let (bob, _bob_jwt, _bob_personal) = seed_user_with_personal(&pool, "bob").await;
        let org = seed_org(&pool, "acme", alice.user_id).await;
        add_member(&pool, org, bob.user_id).await;

        let alice_agent = seed_agent(&pool, org, "alice-bot", alice.user_id).await;
        let bob_agent = seed_agent(&pool, org, "bob-bot", bob.user_id).await;
        let peer = seed_agent(&pool, org, "peer", alice.user_id).await;

        // One friendship proposed by Alice, one by Bob.
        seed_friendship(
            &pool,
            alice_agent,
            peer,
            Some(alice.user_id),
            None,
            "proposed",
            false,
        )
        .await;
        seed_friendship(
            &pool,
            bob_agent,
            peer,
            Some(bob.user_id),
            None,
            "proposed",
            false,
        )
        .await;

        // One capability each.
        seed_capability_attributed(&pool, alice_agent, "do_a", Some(alice.user_id)).await;
        seed_capability_attributed(&pool, bob_agent, "do_b", Some(bob.user_id)).await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/usage/summary?scope=personal")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let by_action = &json["by_action"];
        assert_eq!(by_action["scope"], "personal");
        assert_eq!(by_action["friendships_proposed"], 1);
        assert_eq!(by_action["capabilities_published"], 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn summary_by_action_org_includes_teammate_rows(pool: PgPool) {
        // Same fixture as the Personal test but Alice asks for Org
        // scope and sees both her + Bob's rows.
        let (alice, alice_jwt, _alice_personal) = seed_user_with_personal(&pool, "alice").await;
        let (bob, _bob_jwt, _bob_personal) = seed_user_with_personal(&pool, "bob").await;
        let org = seed_org(&pool, "acme", alice.user_id).await;
        add_member(&pool, org, bob.user_id).await;

        let alice_agent = seed_agent(&pool, org, "alice-bot", alice.user_id).await;
        let bob_agent = seed_agent(&pool, org, "bob-bot", bob.user_id).await;
        let peer = seed_agent(&pool, org, "peer", alice.user_id).await;

        seed_friendship(
            &pool,
            alice_agent,
            peer,
            Some(alice.user_id),
            None,
            "proposed",
            false,
        )
        .await;
        seed_friendship(
            &pool,
            bob_agent,
            peer,
            Some(bob.user_id),
            None,
            "proposed",
            false,
        )
        .await;

        seed_capability_attributed(&pool, alice_agent, "do_a", Some(alice.user_id)).await;
        seed_capability_attributed(&pool, bob_agent, "do_b", Some(bob.user_id)).await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    // Default scope (no query param) is Org.
                    .uri("/v1/usage/summary")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let by_action = &json["by_action"];
        assert_eq!(by_action["scope"], "org");
        assert_eq!(by_action["friendships_proposed"], 2);
        // The three agents seeded above (alice_agent, bob_agent, peer)
        // all sit in the shared org account. Alice's personal account
        // has no agents.
        assert_eq!(by_action["agents_registered"], 3);
        assert_eq!(by_action["capabilities_published"], 2);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn summary_by_action_friendship_transitions(pool: PgPool) {
        // Three friendships in different terminal states — verify the
        // accepted / rejected / cancelled buckets resolve correctly and
        // grants_revoked picks up the revoked-at row.
        let (alice, alice_jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let a = seed_agent(&pool, personal, "a", alice.user_id).await;
        let b = seed_agent(&pool, personal, "b", alice.user_id).await;
        let c = seed_agent(&pool, personal, "c", alice.user_id).await;
        let d = seed_agent(&pool, personal, "d", alice.user_id).await;

        seed_friendship(
            &pool,
            a,
            b,
            Some(alice.user_id),
            Some(alice.user_id),
            "accepted",
            true,
        )
        .await;
        seed_friendship(
            &pool,
            a,
            c,
            Some(alice.user_id),
            Some(alice.user_id),
            "rejected",
            true,
        )
        .await;
        seed_friendship(
            &pool,
            a,
            d,
            Some(alice.user_id),
            Some(alice.user_id),
            "cancelled",
            true,
        )
        .await;

        // One active grant + one revoked grant. The revoke is
        // attributed to Alice so Personal scope counts it.
        let cap = seed_capability(&pool, a).await;
        seed_grant(&pool, a, b, cap, Some(alice.user_id), None, false).await;
        seed_grant(
            &pool,
            a,
            c,
            cap,
            Some(alice.user_id),
            Some(alice.user_id),
            true,
        )
        .await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/usage/summary?scope=personal")
                    .header(header::AUTHORIZATION, format!("Bearer {alice_jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let by_action = &json["by_action"];
        assert_eq!(by_action["friendships_accepted"], 1);
        assert_eq!(by_action["friendships_rejected"], 1);
        assert_eq!(by_action["friendships_cancelled"], 1);
        // Both grants were issued by Alice — Personal counts both.
        assert_eq!(by_action["grants_issued"], 2);
        assert_eq!(by_action["grants_revoked"], 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn pairing_usage_404_for_stranger(pool: PgPool) {
        let (owner, _jwt_owner, owner_personal) = seed_user_with_personal(&pool, "owner").await;
        let owner_agent = seed_agent(&pool, owner_personal, "hermes", owner.user_id).await;
        let device_id = seed_approved_device_flow(&pool, owner.user_id, owner_agent).await;

        // Someone else tries to peek.
        let (_stranger, stranger_jwt, _) = seed_user_with_personal(&pool, "stranger").await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/pairings/device_flow/{device_id}/usage"))
                    .header(header::AUTHORIZATION, format!("Bearer {stranger_jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
