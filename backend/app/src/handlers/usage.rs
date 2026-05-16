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
pub struct SummaryResponse {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub total: TotalCount,
    pub by_org: Vec<OrgRollup>,
    pub by_agent: Vec<AgentRollup>,
    pub by_api_key: Vec<ApiKeyRollup>,
    pub by_pair: Vec<PairRollup>,
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
        daily,
    }))
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
        sqlx::query!(
            r#"
            INSERT INTO relay_invocations
                (id, granter_agent_id, grantee_agent_id, capability_id,
                 capability_name, invoked_by_user_id, status, elapsed_ms,
                 api_key_id, minted_jti)
            VALUES ($1, $2, $3, $4, 'do', $5, $6, 0, $7, $8)
            "#,
            Uuid::now_v7(),
            granter_agent,
            caller_agent,
            capability,
            caller_user,
            status,
            api_key_id,
            minted_jti,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_capability(pool: &PgPool, agent: Uuid) -> Uuid {
        let cap = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                 (id, agent_id, name, description, input_schema, output_schema)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb)"#,
            cap,
            agent,
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
