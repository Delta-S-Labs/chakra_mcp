use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use axum::http::StatusCode;
use chakramcp_shared::error::{ApiError, ApiResult};
use chakramcp_shared::scope::{
    grant_allows_agent, manageable_agent_ids, resolve_grant, AgentScopeMode, GrantScope,
};

use crate::auth::{generate_api_key, hash_api_key, key_prefix, AuthUser};
use crate::state::AppState;

/// The caller's agent-management scope when it's narrower than `all`.
///
/// A credential never creates or obtains one wider than itself (round-2 QA
/// N12): without this, a key limited to its own agents could create an
/// unrestricted key, or rotate one and receive the replacement.
pub(crate) async fn narrowed_scope(
    state: &AppState,
    user: &AuthUser,
) -> ApiResult<Option<GrantScope>> {
    let grant = resolve_grant(&state.db, user.jti, user.api_key_id).await?;
    Ok((grant.mode != AgentScopeMode::All).then_some(grant))
}

/// 403 `chk.auth.out_of_scope` for a credential limited to some agents
/// asking for more than that.
pub(crate) fn beyond_scope(what: &str) -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        "chk.auth.out_of_scope",
        format!(
            "this credential manages only some agents, so it can't {what}; \
             use `chakramcp login` or the web app"
        ),
    )
}

#[derive(Debug, Serialize)]
pub struct ApiKeyDto {
    pub id: Uuid,
    pub name: String,
    pub prefix: String,
    pub account_id: Option<Uuid>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Which agents the key may manage: `all`, `own` or `selected` (round-2
    /// QA N17).
    pub agent_scope: String,
    /// The agents a `selected` key may manage.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub selected_agent_ids: Vec<Uuid>,
}

/// A key's agent scope: `all` when it has no scope row (keys from before
/// scopes, and unscoped keys), and a `selected` key's agents.
async fn scope_of(db: &sqlx::PgPool, key: Uuid) -> Result<(String, Vec<Uuid>), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT cs.agent_scope,
                  ARRAY(SELECT csa.agent_id FROM credential_scope_agents csa
                         WHERE csa.credential_scope_id = cs.id
                         ORDER BY csa.agent_id) AS "selected!"
             FROM credential_scopes cs
            WHERE cs.cred_kind = 'api_key' AND cs.cred_ref = $1::uuid::text"#,
        key,
    )
    .fetch_optional(db)
    .await?;
    Ok(row
        .map(|r| (r.agent_scope, r.selected))
        .unwrap_or_else(|| ("all".to_owned(), Vec::new())))
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    /// Optional scope — if set, the key only authenticates inside this account.
    pub account_id: Option<Uuid>,
    /// TTL in days. Null = never expires.
    pub expires_in_days: Option<i64>,
    /// Agent-management scope for this key (scoped-agent-grants): "all"
    /// (default — unrestricted), "own" (only agents created with this key),
    /// or "selected".
    #[serde(default)]
    pub agent_scope: Option<String>,
    /// Agent ids when `agent_scope == "selected"`. Each must be an agent in
    /// an account the key's owner belongs to.
    #[serde(default)]
    pub selected_agent_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub api_key: ApiKeyDto,
    /// Plaintext — shown to the user exactly once. Never stored.
    pub plaintext: String,
}

// ─────────────────────────────────────────────────────────
// GET /v1/api-keys
// ─────────────────────────────────────────────────────────
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<ApiKeyDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT k.id, k.name, k.key_prefix, k.account_id, k.last_used_at, k.expires_at,
               k.revoked_at, k.created_at,
               COALESCE(cs.agent_scope, 'all') AS "agent_scope!",
               ARRAY(SELECT csa.agent_id FROM credential_scope_agents csa
                      WHERE csa.credential_scope_id = cs.id
                      ORDER BY csa.agent_id) AS "selected_agent_ids!"
        FROM api_keys k
        LEFT JOIN credential_scopes cs
               ON cs.cred_kind = 'api_key' AND cs.cred_ref = k.id::text
        WHERE k.user_id = $1
        ORDER BY k.created_at DESC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| ApiKeyDto {
                id: r.id,
                name: r.name,
                prefix: r.key_prefix,
                account_id: r.account_id,
                last_used_at: r.last_used_at,
                expires_at: r.expires_at,
                revoked_at: r.revoked_at,
                created_at: r.created_at,
                agent_scope: r.agent_scope,
                selected_agent_ids: r.selected_agent_ids,
            })
            .collect(),
    ))
}

// ─────────────────────────────────────────────────────────
// POST /v1/api-keys
// ─────────────────────────────────────────────────────────
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> ApiResult<Json<CreateApiKeyResponse>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::InvalidRequest("name is required".into()));
    }
    if let Some(d) = req.expires_in_days {
        if !(1..=3650).contains(&d) {
            return Err(ApiError::InvalidRequest(
                "expires_in_days must be between 1 and 3650".into(),
            ));
        }
    }

    // If a scope account is requested, verify the user is in it.
    if let Some(account_id) = req.account_id {
        let ok = sqlx::query!(
            r#"SELECT 1 as one FROM account_memberships WHERE user_id = $1 AND account_id = $2 LIMIT 1"#,
            user.user_id,
            account_id,
        )
        .fetch_optional(&state.db)
        .await?
        .is_some();
        if !ok {
            return Err(ApiError::Forbidden);
        }
    }

    // A credential limited to some agents (a pairing token, a scoped key or
    // client) never creates a wider key (round-2 QA N12). Asked for no scope,
    // its key covers the agents it manages now, so a paired agent can make
    // itself a key for the SDK.
    let caller = narrowed_scope(&state, &user).await?;
    let inherited = match (&caller, req.agent_scope.as_deref()) {
        (Some(c), None) => {
            let ids = manageable_agent_ids(&state.db, c)
                .await?
                .unwrap_or_default();
            Some(
                sqlx::query_scalar!(
                    r#"
                    SELECT a.id
                    FROM agents a
                    JOIN account_memberships m ON m.account_id = a.account_id
                    WHERE m.user_id = $1 AND a.id = ANY($2) AND a.tombstoned_at IS NULL
                    "#,
                    user.user_id,
                    &ids,
                )
                .fetch_all(&state.db)
                .await?,
            )
        }
        _ => None,
    };

    // Agent-management scope for this key (scoped-agent-grants). Validate the
    // mode and, for 'selected', that every named agent is in an account the
    // key's owner belongs to.
    let agent_scope = match &inherited {
        Some(ids) if ids.is_empty() => "own",
        Some(_) => "selected",
        None => req.agent_scope.as_deref().unwrap_or("all"),
    };
    if !matches!(agent_scope, "all" | "own" | "selected") {
        return Err(ApiError::InvalidRequest(
            "agent_scope must be all|own|selected".into(),
        ));
    }
    let mut selected_ids = match inherited {
        Some(ids) => ids,
        None => req.selected_agent_ids.clone().unwrap_or_default(),
    };
    selected_ids.sort();
    selected_ids.dedup();
    if agent_scope == "selected" {
        if selected_ids.is_empty() {
            return Err(ApiError::InvalidRequest(
                "selected_agent_ids is required when agent_scope=selected".into(),
            ));
        }
        let in_scope = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "n!"
            FROM agents a
            JOIN account_memberships m ON m.account_id = a.account_id
            WHERE m.user_id = $1 AND a.id = ANY($2)
            "#,
            user.user_id,
            &selected_ids,
        )
        .fetch_one(&state.db)
        .await?;
        if in_scope as usize != selected_ids.len() {
            return Err(ApiError::InvalidRequest(
                "selected_agent_ids must all be agents in your accounts".into(),
            ));
        }
    }

    // Asked for explicitly, a limited credential's key can be `own` (it
    // starts with no agents) or `selected` within its own agents, never `all`.
    if let Some(caller) = &caller {
        if agent_scope == "all" {
            return Err(beyond_scope("create a key for every agent"));
        }
        for aid in &selected_ids {
            if !grant_allows_agent(&state.db, caller, *aid).await? {
                return Err(beyond_scope("create a key for an agent it can't manage"));
            }
        }
    }

    let plaintext = generate_api_key();
    let key_hash = hash_api_key(&plaintext);
    let prefix = key_prefix(&plaintext);
    let id = Uuid::now_v7();
    let expires_at = req.expires_in_days.map(|d| Utc::now() + Duration::days(d));

    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query!(
        r#"
        INSERT INTO api_keys (id, user_id, account_id, name, key_hash, key_prefix, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, name, key_prefix, account_id, last_used_at, expires_at, revoked_at, created_at
        "#,
        id,
        user.user_id,
        req.account_id,
        req.name,
        key_hash,
        prefix,
        expires_at,
    )
    .fetch_one(&mut *tx)
    .await?;

    // Bind the key's agent-management scope (migration 0027), keyed by the
    // key id. 'all' writes no row — resolve_grant treats a missing row as
    // unrestricted, matching the relay's api_key path (cred_ref = key id).
    if agent_scope != "all" {
        let scope_id = Uuid::now_v7();
        sqlx::query!(
            r#"
            INSERT INTO credential_scopes (id, cred_kind, cred_ref, user_id, agent_scope)
            VALUES ($1, 'api_key', $2, $3, $4)
            "#,
            scope_id,
            inserted.id.to_string(),
            user.user_id,
            agent_scope,
        )
        .execute(&mut *tx)
        .await?;
        if agent_scope == "selected" {
            for aid in &selected_ids {
                sqlx::query!(
                    r#"
                    INSERT INTO credential_scope_agents (credential_scope_id, agent_id)
                    VALUES ($1, $2)
                    ON CONFLICT DO NOTHING
                    "#,
                    scope_id,
                    aid,
                )
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    let (agent_scope, selected_agent_ids) = scope_of(&state.db, inserted.id).await?;

    Ok(Json(CreateApiKeyResponse {
        api_key: ApiKeyDto {
            id: inserted.id,
            name: inserted.name,
            prefix: inserted.key_prefix,
            account_id: inserted.account_id,
            last_used_at: inserted.last_used_at,
            expires_at: inserted.expires_at,
            revoked_at: inserted.revoked_at,
            created_at: inserted.created_at,
            agent_scope,
            selected_agent_ids,
        },
        plaintext,
    }))
}

// ─────────────────────────────────────────────────────────
// DELETE /v1/api-keys/:id — revoke
// ─────────────────────────────────────────────────────────
pub async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    if user.api_key_id != Some(id) && narrowed_scope(&state, &user).await?.is_some() {
        return Err(beyond_scope("revoke another credential"));
    }
    let result = sqlx::query!(
        r#"
        UPDATE api_keys
        SET revoked_at = now()
        WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
        "#,
        id,
        user.user_id,
    )
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ─────────────────────────────────────────────────────────
// POST /v1/api-keys/:id/rotate — mint a replacement, retire the old
//
// Same shape as `create`: response contains the freshly-minted DTO and
// the one-and-only plaintext, never stored. The old key gets
// `revoked_at = now()`. We do this in a single transaction so a
// crash mid-rotation can't leave the user with two live keys (or
// neither).
//
// `name` + `account_id` carry over from the old key — the rotation is
// conceptually "this credential keeps its identity, only the secret
// changes".
// ─────────────────────────────────────────────────────────
pub async fn rotate(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<(axum::http::StatusCode, Json<CreateApiKeyResponse>)> {
    // Rotating hands back the key's replacement, so a limited credential may
    // rotate only itself.
    if user.api_key_id != Some(id) && narrowed_scope(&state, &user).await?.is_some() {
        return Err(beyond_scope("rotate another credential"));
    }
    let mut tx = state.db.begin().await?;

    // Lock the source row so two concurrent rotations can't both
    // succeed.
    let old = sqlx::query!(
        r#"
        SELECT id, user_id, account_id, name, expires_at, revoked_at
        FROM api_keys
        WHERE id = $1 AND user_id = $2
        FOR UPDATE
        "#,
        id,
        user.user_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    if old.revoked_at.is_some() {
        return Err(ApiError::Conflict("api key is already revoked".into()));
    }

    // Mint the replacement.
    let plaintext = generate_api_key();
    let key_hash = hash_api_key(&plaintext);
    let prefix = key_prefix(&plaintext);
    let new_id = Uuid::now_v7();

    let inserted = sqlx::query!(
        r#"
        INSERT INTO api_keys (id, user_id, account_id, name, key_hash, key_prefix, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, name, key_prefix, account_id, last_used_at, expires_at, revoked_at, created_at
        "#,
        new_id,
        user.user_id,
        old.account_id,
        old.name,
        key_hash,
        prefix,
        old.expires_at,
    )
    .fetch_one(&mut *tx)
    .await?;

    // Carry the key's agent-management scope over to the new id. The
    // scope row (migration 0027) is keyed by the key id in `cred_ref`,
    // so without this the rotated key would resolve to no row — which
    // `resolve_grant` treats as unrestricted `all`, silently widening a
    // deliberately-narrowed `own`/`selected` key back to full access.
    // Re-pointing (rather than copying) preserves the `selected` set,
    // which FKs the unchanged scope id. `all` keys have no row, so this
    // matches zero rows and is a no-op for them.
    sqlx::query!(
        r#"
        UPDATE credential_scopes
        SET cred_ref = $1
        WHERE cred_kind = 'api_key' AND cred_ref = $2
        "#,
        new_id.to_string(),
        old.id.to_string(),
    )
    .execute(&mut *tx)
    .await?;
    // And the agents it created: an `own` key finds its agents by
    // `created_by_api_key_id`, so they move to the new id too.
    sqlx::query!(
        r#"UPDATE agents SET created_by_api_key_id = $1 WHERE created_by_api_key_id = $2"#,
        new_id,
        old.id,
    )
    .execute(&mut *tx)
    .await?;
    // And the client agent the relay made for it (MCP and discovery spec
    // §6.4), so the new key keeps calling as it.
    sqlx::query!(
        r#"UPDATE default_agents SET key = $1 WHERE key = $2"#,
        format!("api_key:{new_id}"),
        format!("api_key:{}", old.id),
    )
    .execute(&mut *tx)
    .await?;

    // Retire the predecessor. Once this commits, any in-flight request
    // using the old plaintext starts failing at the auth extractor.
    sqlx::query!(
        r#"UPDATE api_keys SET revoked_at = now() WHERE id = $1"#,
        old.id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    let (agent_scope, selected_agent_ids) = scope_of(&state.db, inserted.id).await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            api_key: ApiKeyDto {
                id: inserted.id,
                name: inserted.name,
                prefix: inserted.key_prefix,
                account_id: inserted.account_id,
                last_used_at: inserted.last_used_at,
                expires_at: inserted.expires_at,
                revoked_at: inserted.revoked_at,
                created_at: inserted.created_at,
                agent_scope,
                selected_agent_ids,
            },
            plaintext,
        }),
    ))
}

// ─────────────────────────────────────────────────────────
// GET /v1/api-keys/:id/usage?from=&to=
// ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    /// ISO-8601 timestamp. Defaults to `now - 30 days`.
    pub from: Option<DateTime<Utc>>,
    /// ISO-8601 timestamp. Defaults to `now`.
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CapabilityCount {
    pub capability_name: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct AgentCount {
    pub agent_id: Uuid,
    pub agent_slug: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct DailyCount {
    pub date: chrono::NaiveDate,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub key_id: Uuid,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub total_requests: i64,
    pub by_capability_type: Vec<CapabilityCount>,
    pub by_agent: Vec<AgentCount>,
    pub daily: Vec<DailyCount>,
}

pub async fn usage(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<UsageQuery>,
) -> ApiResult<Json<UsageResponse>> {
    // Confirm the key belongs to the caller. We don't require it to be
    // live — usage data for revoked keys is the most interesting case
    // (post-hoc forensics on a key the user just retired).
    let key = sqlx::query!(
        r#"SELECT id FROM api_keys WHERE id = $1 AND user_id = $2"#,
        id,
        user.user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let to = q.to.unwrap_or_else(Utc::now);
    let from = q.from.unwrap_or_else(|| to - Duration::days(30));
    if from >= to {
        return Err(ApiError::InvalidRequest(
            "`from` must be strictly before `to`".into(),
        ));
    }

    let total = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) AS "count!"
        FROM relay_invocations
        WHERE api_key_id = $1
          AND created_at >= $2
          AND created_at < $3
        "#,
        key.id,
        from,
        to,
    )
    .fetch_one(&state.db)
    .await?;

    // Top 5 capability names + an "(other)" bucket so the UI can render
    // a stacked bar without paging. 5 is an arbitrary but UI-friendly
    // cutoff; the frontend can always page through `relay_invocations`
    // directly if it wants the long tail.
    let top_caps = sqlx::query!(
        r#"
        SELECT capability_name, COUNT(*) AS "count!"
        FROM relay_invocations
        WHERE api_key_id = $1
          AND created_at >= $2
          AND created_at < $3
        GROUP BY capability_name
        ORDER BY COUNT(*) DESC
        LIMIT 5
        "#,
        key.id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let top_sum: i64 = top_caps.iter().map(|r| r.count).sum();
    let mut by_capability_type: Vec<CapabilityCount> = top_caps
        .into_iter()
        .map(|r| CapabilityCount {
            capability_name: r.capability_name,
            count: r.count,
        })
        .collect();
    if total > top_sum {
        by_capability_type.push(CapabilityCount {
            capability_name: "(other)".into(),
            count: total - top_sum,
        });
    }

    // Per-agent breakdown — we report the *grantee* (the agent that
    // actually ran the capability). LEFT JOIN so a tombstoned/deleted
    // agent still contributes its row; we drop those silently because
    // the frontend can't render an agent without a slug.
    let by_agent_rows = sqlx::query!(
        r#"
        SELECT
            i.grantee_agent_id AS "agent_id!",
            a.slug AS "agent_slug!",
            COUNT(*) AS "count!"
        FROM relay_invocations i
        JOIN agents a ON a.id = i.grantee_agent_id
        WHERE i.api_key_id = $1
          AND i.created_at >= $2
          AND i.created_at < $3
        GROUP BY i.grantee_agent_id, a.slug
        ORDER BY COUNT(*) DESC
        LIMIT 20
        "#,
        key.id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let by_agent: Vec<AgentCount> = by_agent_rows
        .into_iter()
        .map(|r| AgentCount {
            agent_id: r.agent_id,
            agent_slug: r.agent_slug,
            count: r.count,
        })
        .collect();

    // Daily sparkline. Postgres' date_trunc returns timestamp; cast to
    // date so the JSON serialises as `"YYYY-MM-DD"`.
    let daily_rows = sqlx::query!(
        r#"
        SELECT
            date_trunc('day', created_at)::date AS "date!",
            COUNT(*) AS "count!"
        FROM relay_invocations
        WHERE api_key_id = $1
          AND created_at >= $2
          AND created_at < $3
        GROUP BY 1
        ORDER BY 1 ASC
        "#,
        key.id,
        from,
        to,
    )
    .fetch_all(&state.db)
    .await?;
    let daily: Vec<DailyCount> = daily_rows
        .into_iter()
        .map(|r| DailyCount {
            date: r.date,
            count: r.count,
        })
        .collect();

    Ok(Json(UsageResponse {
        key_id: key.id,
        from,
        to,
        total_requests: total,
        by_capability_type,
        by_agent,
        daily,
    }))
}

#[cfg(test)]
mod tests {
    use crate::tests_support::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    // Creating a key with a 'selected' agent scope materialises a
    // credential_scopes row keyed by the key id (+ the agent link), which is
    // exactly what the relay's api_key path then enforces. Runtime queries
    // keep this out of the .sqlx cache.
    #[sqlx::test(migrations = "../migrations")]
    async fn create_with_selected_scope_writes_credential_scope(pool: PgPool) {
        let (user, jwt, account) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, account, "a1", user.user_id).await;
        let state = crate::AppState::new(pool.clone(), test_config());

        let body = serde_json::json!({
            "name": "ci key",
            "agent_scope": "selected",
            "selected_agent_ids": [agent_id],
        });
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/api-keys")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "create should succeed");
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let key_id = v["api_key"]["id"].as_str().unwrap();
        // The key says what it may manage (round-2 QA N17), on create and in
        // the list.
        assert_eq!(v["api_key"]["agent_scope"], "selected");
        assert_eq!(
            v["api_key"]["selected_agent_ids"],
            serde_json::json!([agent_id])
        );
        let listed = crate::router(crate::AppState::new(pool.clone(), test_config()))
            .oneshot(
                Request::builder()
                    .uri("/v1/api-keys")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let listed: serde_json::Value =
            serde_json::from_slice(&listed.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(listed[0]["agent_scope"], "selected", "{listed}");
        assert_eq!(
            listed[0]["selected_agent_ids"],
            serde_json::json!([agent_id])
        );

        let (scope,): (String,) = sqlx::query_as(
            "SELECT agent_scope FROM credential_scopes WHERE cred_kind = 'api_key' AND cred_ref = $1",
        )
        .bind(key_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(scope, "selected");

        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM credential_scope_agents csa \
             JOIN credential_scopes cs ON cs.id = csa.credential_scope_id \
             WHERE cs.cred_ref = $1 AND csa.agent_id = $2",
        )
        .bind(key_id)
        .bind(agent_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rotate_retires_old_and_returns_new_plaintext(pool: PgPool) {
        let (user, jwt, _personal) = seed_user_with_personal(&pool, "alice").await;
        let (key_id, old_plaintext) = seed_api_key(&pool, user.user_id, "ci key").await;

        let state = crate::AppState::new(pool.clone(), test_config());
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/api-keys/{key_id}/rotate"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let new_plaintext = json["plaintext"].as_str().unwrap().to_string();
        assert!(new_plaintext.starts_with("ck_"));
        assert_ne!(new_plaintext, old_plaintext);

        // The new plaintext authenticates; the old does not.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {new_plaintext}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {old_plaintext}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    // Regression (scoped-grants audit, F1): rotating a scoped key must CARRY
    // its agent-management scope to the new id. The scope row is keyed by the
    // key id, so minting a new id without moving the row leaves the new key
    // with no row — which resolve_grant treats as unrestricted `all`,
    // silently widening a deliberately-narrowed `selected`/`own` key.
    #[sqlx::test(migrations = "../migrations")]
    async fn rotate_preserves_agent_scope(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, personal, "a1", user.user_id).await;
        let state = crate::AppState::new(pool.clone(), test_config());

        // Create a key scoped to exactly agent_id.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/api-keys")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "ci key",
                            "agent_scope": "selected",
                            "selected_agent_ids": [agent_id],
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success());
        let v: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let old_id = v["api_key"]["id"].as_str().unwrap().to_string();

        // Rotate it.
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/api-keys/{old_id}/rotate"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let new_id = v["api_key"]["id"].as_str().unwrap().to_string();
        assert_ne!(new_id, old_id);

        // The scope row moved to the new id; the old id keeps none.
        let old_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM credential_scopes WHERE cred_kind='api_key' AND cred_ref=$1",
        )
        .bind(&old_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(old_rows, 0, "old key id must not keep a scope row");

        let scope: Option<(String,)> = sqlx::query_as(
            "SELECT agent_scope FROM credential_scopes WHERE cred_kind='api_key' AND cred_ref=$1",
        )
        .bind(&new_id)
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(
            scope.map(|s| s.0),
            Some("selected".to_string()),
            "rotated key must keep its 'selected' scope, not widen to all"
        );

        // ...and the selected-set survives the move.
        let enrolled: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM credential_scope_agents csa \
             JOIN credential_scopes cs ON cs.id = csa.credential_scope_id \
             WHERE cs.cred_ref=$1 AND csa.agent_id=$2",
        )
        .bind(&new_id)
        .bind(agent_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(enrolled, 1, "selected-set must survive rotation");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn usage_returns_zero_when_no_invocations(pool: PgPool) {
        let (user, jwt, _personal) = seed_user_with_personal(&pool, "alice").await;
        let (key_id, _plaintext) = seed_api_key(&pool, user.user_id, "ci key").await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/api-keys/{key_id}/usage"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total_requests"], 0);
        assert_eq!(json["key_id"], serde_json::json!(key_id));
    }

    /// An `own` key finds its agents by `created_by_api_key_id`, so
    /// rotating it moves them to the new key id rather than stranding them.
    #[sqlx::test(migrations = "../migrations")]
    async fn rotate_keeps_an_own_keys_agents(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, personal, "a1", user.user_id).await;
        let state = crate::AppState::new(pool.clone(), test_config());

        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/api-keys")
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "name": "own key", "agent_scope": "own" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success());
        let v: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let old_id: uuid::Uuid = v["api_key"]["id"].as_str().unwrap().parse().unwrap();
        sqlx::query("UPDATE agents SET created_by_api_key_id = $1 WHERE id = $2")
            .bind(old_id)
            .bind(agent_id)
            .execute(&pool)
            .await
            .unwrap();
        // The client agent the relay made for the key (MCP spec §6.4).
        sqlx::query(
            "INSERT INTO default_agents (key, user_id, agent_id)
             SELECT 'api_key:' || id, user_id, $2 FROM api_keys WHERE id = $1",
        )
        .bind(old_id)
        .bind(agent_id)
        .execute(&pool)
        .await
        .unwrap();

        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/api-keys/{old_id}/rotate"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let v: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let new_id: uuid::Uuid = v["api_key"]["id"].as_str().unwrap().parse().unwrap();

        let by: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT created_by_api_key_id FROM agents WHERE id = $1")
                .bind(agent_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(by, Some(new_id));
        let default: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT agent_id FROM default_agents WHERE key = $1")
                .bind(format!("api_key:{new_id}"))
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(
            default,
            Some(agent_id),
            "the default agent moves with the key"
        );
    }

    async fn send(
        pool: &PgPool,
        method: &str,
        uri: &str,
        bearer: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let state = crate::AppState::new(pool.clone(), test_config());
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .header(header::CONTENT_TYPE, "application/json");
        let req = match body {
            Some(b) => builder.body(Body::from(b.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        let res = crate::router(state).oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// A key limited to `agent_id`, created through the API.
    async fn limited_key(pool: &PgPool, jwt: &str, agent_id: uuid::Uuid) -> (String, String) {
        let (status, v) = send(
            pool,
            "POST",
            "/v1/api-keys",
            jwt,
            Some(serde_json::json!({
                "name": "agent key",
                "agent_scope": "selected",
                "selected_agent_ids": [agent_id],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        (
            v["api_key"]["id"].as_str().unwrap().to_string(),
            v["plaintext"].as_str().unwrap().to_string(),
        )
    }

    // Round-2 QA N12: a credential limited to some agents can't create a key
    // wider than itself. `own` (starts with no agents) and `selected` within
    // its own agents are fine.
    #[sqlx::test(migrations = "../migrations")]
    async fn limited_key_cannot_create_a_wider_key(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let mine = seed_agent(&pool, personal, "mine", user.user_id).await;
        let other = seed_agent(&pool, personal, "other", user.user_id).await;
        let (_id, key) = limited_key(&pool, &jwt, mine).await;

        let (status, v) = send(
            &pool,
            "POST",
            "/v1/api-keys",
            &key,
            Some(serde_json::json!({ "name": "wide", "agent_scope": "all" })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        assert_eq!(v["error"]["code"], "chk.auth.out_of_scope", "{v}");

        // Asked for no scope, the key covers the same agents as its creator.
        let (status, v) = send(
            &pool,
            "POST",
            "/v1/api-keys",
            &key,
            Some(serde_json::json!({ "name": "sdk" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let new_id = v["api_key"]["id"].as_str().unwrap().to_string();
        let covered: Vec<uuid::Uuid> = sqlx::query_scalar(
            "SELECT csa.agent_id FROM credential_scope_agents csa \
             JOIN credential_scopes cs ON cs.id = csa.credential_scope_id \
             WHERE cs.cred_kind = 'api_key' AND cs.cred_ref = $1 AND cs.agent_scope = 'selected'",
        )
        .bind(&new_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(covered, vec![mine]);

        let (status, _) = send(
            &pool,
            "POST",
            "/v1/api-keys",
            &key,
            Some(serde_json::json!({
                "name": "other",
                "agent_scope": "selected",
                "selected_agent_ids": [other],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        for body in [
            serde_json::json!({ "name": "own", "agent_scope": "own" }),
            serde_json::json!({
                "name": "same",
                "agent_scope": "selected",
                "selected_agent_ids": [mine],
            }),
        ] {
            let (status, v) = send(&pool, "POST", "/v1/api-keys", &key, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{v}");
        }

        // The unrestricted web session still creates unrestricted keys.
        let (status, _) = send(
            &pool,
            "POST",
            "/v1/api-keys",
            &jwt,
            Some(serde_json::json!({ "name": "wide" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    // Round-2 QA N12: rotating returns the replacement key, so a limited
    // credential may rotate (and revoke) only itself.
    #[sqlx::test(migrations = "../migrations")]
    async fn limited_key_rotates_and_revokes_only_itself(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let mine = seed_agent(&pool, personal, "mine", user.user_id).await;
        let (wide_id, _wide) = seed_api_key(&pool, user.user_id, "wide").await;
        let (own_id, key) = limited_key(&pool, &jwt, mine).await;

        let (status, _) = send(
            &pool,
            "POST",
            &format!("/v1/api-keys/{wide_id}/rotate"),
            &key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = send(
            &pool,
            "DELETE",
            &format!("/v1/api-keys/{wide_id}"),
            &key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = send(
            &pool,
            "POST",
            &format!("/v1/pairings/api_key/{wide_id}/revoke"),
            &key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, v) = send(
            &pool,
            "POST",
            &format!("/v1/api-keys/{own_id}/rotate"),
            &key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
    }
}
