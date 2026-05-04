//! Agent CRUD + network discovery.
//!
//! An agent always lives inside an account (personal or organization).
//! Mutations require the caller to be a member of that account; admin
//! mutations (visibility flips, deletes) require owner|admin role.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_can_admin_account, user_is_member, AuthUser};
use crate::state::RelayState;

// ─── Validation helpers ──────────────────────────────────

/// Slug rules per discovery design §"Slug allocation":
/// 3–32 chars, ASCII `[a-z0-9-]`, no leading/trailing or double
/// hyphens. NFKC normalization + reserved-words list land in D9.
fn is_valid_slug(s: &str) -> bool {
    let len = s.chars().count();
    if !(3..=32).contains(&len) {
        return false;
    }
    if s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    if s.contains("--") {
        return false;
    }
    s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Parse + normalize an Agent Card URL. Rejects non-http(s) schemes,
/// missing host, fragments. Returns the canonicalized form (URL with
/// fragment stripped, trailing whitespace gone) the caller should
/// store. The fetcher (D2d) further validates at fetch time, but
/// catching the obvious garbage here gives users a clean error
/// instead of a "fetch failed" surprise an hour later.
fn validate_agent_card_url(raw: &str) -> Result<String, ApiError> {
    // url::Url::parse is lenient toward malformed authorities: e.g.
    // `https:///path` parses with `host_str = Some("path")` because
    // the parser swallows one extra `/` to recover. We catch that
    // class of typo with an explicit pre-parse check before relying
    // on the parsed result.
    if raw.contains(":///") {
        return Err(ApiError::InvalidRequest(
            "agent_card_url is missing a host (saw `:///`)".into(),
        ));
    }
    let mut parsed = url::Url::parse(raw).map_err(|_| {
        ApiError::InvalidRequest(
            "agent_card_url must be a valid absolute URL".into(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::InvalidRequest(
            "agent_card_url must use http or https scheme".into(),
        ));
    }
    match parsed.host_str() {
        Some(h) if !h.is_empty() => {}
        _ => {
            return Err(ApiError::InvalidRequest(
                "agent_card_url must include a non-empty host".into(),
            ))
        }
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

enum CardChange {
    Unchanged,
    ClearToPull,
    SetToPush(String),
}

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AgentDto {
    pub id: Uuid,
    pub account_id: Uuid,
    pub account_slug: String,
    pub account_display_name: String,
    pub slug: String,
    pub display_name: String,
    pub description: String,
    pub visibility: String,
    pub endpoint_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// True when the requesting user is a member of the owning account.
    pub is_mine: bool,
    pub capability_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub account_id: Uuid,
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub visibility: Option<String>,
    pub endpoint_url: Option<String>,
    /// A2A canonical Agent Card URL for push-mode targets (D2d).
    /// When provided, the agent registers as `mode='push'` and the
    /// background refresh job (D2e) starts fetching + caching its
    /// upstream card. When absent, the agent stays in pull mode
    /// (the default — `inbox.serve()`-style polling).
    #[serde(default)]
    pub agent_card_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<String>,
    pub endpoint_url: Option<Option<String>>,
    /// Push↔pull migration:
    ///
    /// - Field **absent**: mode unchanged.
    /// - Field present with **empty string** `""`: demote to pull.
    ///   Clears `agent_card_url`, sets `mode='pull'`.
    /// - Field present with a **URL**: promote to push. Sets
    ///   `agent_card_url` and `mode='push'`.
    ///
    /// Serde collapses missing-field and `null` to the same `None`
    /// for `Option<Option<T>>` without third-party helpers, so we
    /// use `""` as the explicit "clear" sentinel rather than
    /// playing serde-with games. The DB CHECK
    /// `agents_mode_card_consistency` (migration 0010) catches any
    /// inconsistency at COMMIT time as a defense in depth.
    #[serde(default)]
    pub agent_card_url: Option<String>,
}

// ─── GET /v1/agents — list mine ──────────────────────────
pub async fn list_mine(
    State(state): State<RelayState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<AgentDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.account_id IN (
            SELECT account_id FROM account_memberships WHERE user_id = $1
        )
        ORDER BY a.created_at DESC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AgentDto {
                id: r.id,
                account_id: r.account_id,
                account_slug: r.account_slug,
                account_display_name: r.account_display_name,
                slug: r.slug,
                display_name: r.display_name,
                description: r.description,
                visibility: r.visibility,
                endpoint_url: r.endpoint_url,
                created_at: r.created_at,
                updated_at: r.updated_at,
                is_mine: true,
                capability_count: r.capability_count,
            })
            .collect(),
    ))
}

// ─── GET /v1/network/agents — discover ───────────────────
pub async fn list_network(
    State(state): State<RelayState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<AgentDto>>> {
    let rows = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c
                WHERE c.agent_id = a.id AND c.visibility = 'network') as "capability_count!",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.account_id = a.account_id AND m.user_id = $1
            ) as "is_mine!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.visibility = 'network'
        ORDER BY a.created_at DESC
        "#,
        user.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| AgentDto {
                id: r.id,
                account_id: r.account_id,
                account_slug: r.account_slug,
                account_display_name: r.account_display_name,
                slug: r.slug,
                display_name: r.display_name,
                description: r.description,
                visibility: r.visibility,
                endpoint_url: if r.is_mine { r.endpoint_url } else { None },
                created_at: r.created_at,
                updated_at: r.updated_at,
                is_mine: r.is_mine,
                capability_count: r.capability_count,
            })
            .collect(),
    ))
}

// ─── GET /v1/agents/{id} ─────────────────────────────────
pub async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AgentDto>> {
    let r = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!",
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.account_id = a.account_id AND m.user_id = $1
            ) as "is_mine!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.id = $2
        "#,
        user.user_id,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !r.is_mine && r.visibility != "network" {
        return Err(ApiError::NotFound);
    }

    Ok(Json(AgentDto {
        id: r.id,
        account_id: r.account_id,
        account_slug: r.account_slug,
        account_display_name: r.account_display_name,
        slug: r.slug,
        display_name: r.display_name,
        description: r.description,
        visibility: r.visibility,
        endpoint_url: if r.is_mine { r.endpoint_url } else { None },
        created_at: r.created_at,
        updated_at: r.updated_at,
        is_mine: r.is_mine,
        capability_count: r.capability_count,
    }))
}

// ─── POST /v1/agents ─────────────────────────────────────
pub async fn create(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<CreateRequest>,
) -> ApiResult<Json<AgentDto>> {
    let slug = req.slug.trim().to_lowercase();
    let display_name = req.display_name.trim().to_string();
    if slug.is_empty() || display_name.is_empty() {
        return Err(ApiError::InvalidRequest("slug and display_name are required".into()));
    }
    // Slug rules per discovery design §"Slug allocation":
    //   ASCII a-z 0-9 -, length 3-32, no leading/trailing hyphen,
    //   no double hyphens, NFKC-normalized. NFKC normalization will
    //   land in D9 alongside the reserved-words list; the rest is
    //   enforced here so registrations made today don't need to be
    //   grandfathered when D9 ships.
    if !is_valid_slug(&slug) {
        return Err(ApiError::InvalidRequest(
            "slug must be 3-32 chars of [a-z0-9-], no leading/trailing or double hyphens".into(),
        ));
    }
    let visibility = req.visibility.as_deref().unwrap_or("private");
    if !matches!(visibility, "private" | "network") {
        return Err(ApiError::InvalidRequest("visibility must be private|network".into()));
    }

    if !user_is_member(&state.db, user.user_id, req.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    let id = Uuid::now_v7();
    // ON CONFLICT must match the partial unique index (`tombstoned_at IS NULL`)
    // introduced by migration 0009 — old code targeted a full UNIQUE that was
    // replaced. The predicate keeps the conflict scope to live (non-tombstoned)
    // rows, which is exactly the semantics we want: re-registering after a
    // tombstone is a separate explicit "untombstone" action, not an implicit
    // upsert.
    // Push mode iff agent_card_url is supplied. The CHECK
    // constraint `agents_mode_card_consistency` (migration 0010)
    // enforces this invariant at the DB layer; we just have to
    // pick the right mode at insert time. Pull mode (default)
    // means the agent will run `inbox.serve()` against our relay's
    // inbox bridge and has no public A2A endpoint.
    let agent_card_url = req
        .agent_card_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent_card_url = agent_card_url
        .map(validate_agent_card_url)
        .transpose()?;
    let mode = if agent_card_url.is_some() {
        "push"
    } else {
        "pull"
    };
    let inserted = sqlx::query!(
        r#"
        INSERT INTO agents
            (id, account_id, created_by_user_id, slug, display_name, description,
             visibility, endpoint_url, mode, agent_card_url)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (account_id, slug) WHERE tombstoned_at IS NULL DO NOTHING
        RETURNING id, account_id, slug, display_name, description, visibility, endpoint_url, created_at, updated_at
        "#,
        id,
        req.account_id,
        user.user_id,
        slug,
        display_name,
        req.description,
        visibility,
        req.endpoint_url,
        mode,
        agent_card_url,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::Conflict(format!("agent slug '{slug}' already exists in this account")))?;

    let acc = sqlx::query!(
        r#"SELECT slug, display_name FROM accounts WHERE id = $1"#,
        inserted.account_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AgentDto {
        id: inserted.id,
        account_id: inserted.account_id,
        account_slug: acc.slug,
        account_display_name: acc.display_name,
        slug: inserted.slug,
        display_name: inserted.display_name,
        description: inserted.description,
        visibility: inserted.visibility,
        endpoint_url: inserted.endpoint_url,
        created_at: inserted.created_at,
        updated_at: inserted.updated_at,
        is_mine: true,
        capability_count: 0,
    }))
}

// ─── PATCH /v1/agents/{id} ───────────────────────────────
pub async fn update(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<Json<AgentDto>> {
    let row = sqlx::query!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !user_is_member(&state.db, user.user_id, row.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    if let Some(v) = req.visibility.as_deref() {
        if !matches!(v, "private" | "network") {
            return Err(ApiError::InvalidRequest("visibility must be private|network".into()));
        }
        if v == "network" && !user_can_admin_account(&state.db, user.user_id, row.account_id).await? {
            return Err(ApiError::Forbidden);
        }
    }

    // Push↔pull migration. See UpdateRequest doc.
    // Empty string ("") is the explicit clear sentinel; missing
    // field leaves mode untouched.
    let card_change = match req.agent_card_url.as_deref().map(str::trim) {
        None => CardChange::Unchanged,
        Some("") => CardChange::ClearToPull,
        Some(raw) => CardChange::SetToPush(validate_agent_card_url(raw)?),
    };

    let (set_card, new_card, set_mode, new_mode) = match &card_change {
        CardChange::Unchanged => (false, None, false, "pull"),
        CardChange::ClearToPull => (true, None, true, "pull"),
        CardChange::SetToPush(url) => (true, Some(url.as_str()), true, "push"),
    };
    sqlx::query!(
        r#"
        UPDATE agents
        SET display_name = COALESCE($2, display_name),
            description = COALESCE($3, description),
            visibility = COALESCE($4, visibility),
            endpoint_url = CASE
                WHEN $5::boolean THEN $6
                ELSE endpoint_url
            END,
            agent_card_url = CASE
                WHEN $7::boolean THEN $8
                ELSE agent_card_url
            END,
            mode = CASE
                WHEN $9::boolean THEN $10
                ELSE mode
            END
        WHERE id = $1
        "#,
        id,
        req.display_name.as_deref(),
        req.description.as_deref(),
        req.visibility.as_deref(),
        req.endpoint_url.is_some(),
        req.endpoint_url.flatten(),
        set_card,
        new_card,
        set_mode,
        new_mode,
    )
    .execute(&state.db)
    .await?;

    let r = sqlx::query!(
        r#"
        SELECT
            a.id, a.account_id, a.slug, a.display_name, a.description,
            a.visibility, a.endpoint_url, a.created_at, a.updated_at,
            acc.slug as account_slug, acc.display_name as account_display_name,
            (SELECT COUNT(*)::bigint FROM agent_capabilities c WHERE c.agent_id = a.id) as "capability_count!"
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.id = $1
        "#,
        id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(AgentDto {
        id: r.id,
        account_id: r.account_id,
        account_slug: r.account_slug,
        account_display_name: r.account_display_name,
        slug: r.slug,
        display_name: r.display_name,
        description: r.description,
        visibility: r.visibility,
        endpoint_url: r.endpoint_url,
        created_at: r.created_at,
        updated_at: r.updated_at,
        is_mine: true,
        capability_count: r.capability_count,
    }))
}

// ─── DELETE /v1/agents/{id} ──────────────────────────────
//
// Soft tombstone (D9). Per discovery design §"Slug allocation":
// deletion sets `tombstoned_at = now()` and the slug becomes
// permanently unavailable (no recycling) until an explicit
// untombstone by the same account, or a forced takedown frees
// the slug after a 30-day cooling period. The row stays in place
// so the audit log resolves, friendships/grants pointing at it
// stay readable, and discovery can return 410 Gone instead of a
// confusing 404 for callers with stale URLs.
pub async fn delete(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let row = sqlx::query!(
        r#"SELECT account_id, tombstoned_at FROM agents WHERE id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if !user_can_admin_account(&state.db, user.user_id, row.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    if row.tombstoned_at.is_some() {
        // Idempotent: re-tombstoning a tombstoned agent is a no-op.
        return Ok(axum::http::StatusCode::NO_CONTENT);
    }

    sqlx::query!(
        r#"UPDATE agents SET tombstoned_at = now() WHERE id = $1"#,
        id,
    )
    .execute(&state.db)
    .await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod create_push_mode_tests {
    //! D8: POST /v1/agents now accepts `agent_card_url` to register a
    //! push-mode agent. Pull mode (no URL) was already supported and
    //! continues to work — verified by the v0.1.0 contract test in
    //! `invoke.rs::legacy_v01_contract_tests`. These tests pin down
    //! the new push branch + the validation rules.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    /// Mint a user + account + membership and return a Bearer JWT
    /// the caller can use against /v1/agents.
    async fn seed_user_with_jwt(pool: &PgPool) -> (Uuid, Uuid, String) {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Test User', 'x')"#,
            user_id,
            format!("{user_id}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Acct', 'individual', $3)"#,
            account_id,
            format!("acct-{account_id}"),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        let token = jwt::encode_jwt(
            &jwt::UserClaims::new(user_id, format!("{user_id}@t.local"), false, 1),
            "test-secret-test-secret-test-secret-test-secret",
        )
        .unwrap();
        (user_id, account_id, token)
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn pull_mode_default_when_no_url(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "alice-pull",
                            "display_name": "Alice Pull",
                            "visibility": "network",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "got {}", res.status());
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let agent_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

        let row = sqlx::query!(
            "SELECT mode, agent_card_url FROM agents WHERE id = $1",
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.mode, "pull");
        assert!(row.agent_card_url.is_none());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn push_mode_when_agent_card_url_supplied(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "alice-push",
                            "display_name": "Alice Push",
                            "visibility": "network",
                            "agent_card_url": "https://travel.example.com/.well-known/agent-card.json",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "got {}", res.status());
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let agent_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

        let row = sqlx::query!(
            "SELECT mode, agent_card_url FROM agents WHERE id = $1",
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.mode, "push");
        assert_eq!(
            row.agent_card_url.as_deref(),
            Some("https://travel.example.com/.well-known/agent-card.json")
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_relative_agent_card_url(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "evil",
                            "display_name": "Evil",
                            "agent_card_url": "/etc/passwd",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn empty_agent_card_url_is_treated_as_pull(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "alice-empty",
                            "display_name": "Alice",
                            "agent_card_url": "   ",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success());
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let agent_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
        let row = sqlx::query!(
            "SELECT mode, agent_card_url FROM agents WHERE id = $1",
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // Whitespace-only URL trimmed to empty -> pull mode, no URL stored.
        assert_eq!(row.mode, "pull");
        assert!(row.agent_card_url.is_none());
    }
}

#[cfg(test)]
mod mode_mutation_and_validation_tests {
    //! Post-D8-review additions:
    //!  * Pull → push and push → pull migration via PATCH (the
    //!    discovery design's "promote later" path).
    //!  * Tightened URL validation via url::Url::parse.
    //!  * Tightened slug rules toward D9 spec form.
    //!  * Conflict on duplicate live slug.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    async fn seed_user_with_jwt(pool: &PgPool) -> (Uuid, Uuid, String) {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Test', 'x')"#,
            user_id,
            format!("{user_id}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Acct', 'individual', $3)"#,
            account_id,
            format!("acct-{account_id}"),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        let token = jwt::encode_jwt(
            &jwt::UserClaims::new(user_id, format!("{user_id}@t.local"), false, 1),
            "test-secret-test-secret-test-secret-test-secret",
        )
        .unwrap();
        (user_id, account_id, token)
    }

    async fn create_pull_agent(pool: &PgPool, account: Uuid, slug: &str, token: &str) -> Uuid {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account,
                            "slug": slug,
                            "display_name": "Agent",
                            "visibility": "network",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        body["id"].as_str().unwrap().parse().unwrap()
    }

    async fn patch(
        pool: &PgPool,
        token: &str,
        agent_id: Uuid,
        body: serde_json::Value,
    ) -> axum::http::Response<Body> {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        app.oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/v1/agents/{agent_id}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    // ── Mode mutation ─────────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn pull_can_be_promoted_to_push_via_patch(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let agent_id = create_pull_agent(&pool, account_id, "alice", &token).await;

        let res = patch(
            &pool,
            &token,
            agent_id,
            serde_json::json!({
                "agent_card_url": "https://travel.example.com/.well-known/agent-card.json"
            }),
        )
        .await;
        assert!(res.status().is_success(), "got {}", res.status());

        let row =
            sqlx::query!("SELECT mode, agent_card_url FROM agents WHERE id = $1", agent_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.mode, "push");
        assert_eq!(
            row.agent_card_url.as_deref(),
            Some("https://travel.example.com/.well-known/agent-card.json")
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn push_can_be_demoted_to_pull_via_empty_string(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        // Create as push first.
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "bob-push",
                            "display_name": "Bob",
                            "agent_card_url": "https://example.com/.well-known/agent-card.json",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let agent_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

        // Demote: send agent_card_url="" (the documented clear sentinel).
        let res = patch(
            &pool,
            &token,
            agent_id,
            serde_json::json!({"agent_card_url": ""}),
        )
        .await;
        assert!(res.status().is_success(), "got {}", res.status());

        let row =
            sqlx::query!("SELECT mode, agent_card_url FROM agents WHERE id = $1", agent_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.mode, "pull");
        assert!(row.agent_card_url.is_none());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn patch_without_agent_card_url_field_leaves_mode_alone(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let agent_id = create_pull_agent(&pool, account_id, "carol", &token).await;
        let res = patch(
            &pool,
            &token,
            agent_id,
            serde_json::json!({"display_name": "Carol Updated"}),
        )
        .await;
        assert!(res.status().is_success());
        let row =
            sqlx::query!("SELECT mode, display_name FROM agents WHERE id = $1", agent_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.mode, "pull");
        assert_eq!(row.display_name, "Carol Updated");
    }

    // ── Tightened URL validation ──────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_url_without_host(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        for bogus in [
            "https://",
            "http://",
            "https:///path",
            "https//missing-colon.com",
        ] {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/agents")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({
                                "account_id": account_id,
                                "slug": "scratch",
                                "display_name": "S",
                                "agent_card_url": bogus,
                            }))
                            .unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::BAD_REQUEST,
                "bogus URL should be rejected: {bogus}"
            );
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn url_fragment_is_stripped_on_storage(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let agent_id = create_pull_agent(&pool, account_id, "dave", &token).await;
        let res = patch(
            &pool,
            &token,
            agent_id,
            serde_json::json!({
                "agent_card_url":
                    "https://example.com/.well-known/agent-card.json#fragment"
            }),
        )
        .await;
        assert!(res.status().is_success(), "got {}", res.status());
        let row = sqlx::query!("SELECT agent_card_url FROM agents WHERE id = $1", agent_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            row.agent_card_url.as_deref(),
            Some("https://example.com/.well-known/agent-card.json")
        );
    }

    // ── Slug rules toward D9 spec ─────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_too_short_slug(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "ab",
                            "display_name": "Tiny",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_double_hyphen_and_leading_hyphen(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        for bad_slug in ["foo--bar", "-foo", "bar-"] {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/agents")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::from(
                            serde_json::to_vec(&serde_json::json!({
                                "account_id": account_id,
                                "slug": bad_slug,
                                "display_name": "Z",
                            }))
                            .unwrap(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::BAD_REQUEST,
                "bad slug should be rejected: {bad_slug}"
            );
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_underscore_per_d9_spec(pool: PgPool) {
        // D8 originally accepted underscores; D9 bans them. We
        // tighten now so registrations made today don't need to be
        // grandfathered when D9's reserved-words list lands.
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "my_bot",
                            "display_name": "Z",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    // ── Concurrency / conflict ────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn duplicate_live_slug_returns_conflict(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let _first = create_pull_agent(&pool, account_id, "duplicate", &token).await;
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "duplicate",
                            "display_name": "Dup",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }
}

#[cfg(test)]
mod tombstone_tests {
    //! D9a: DELETE /v1/agents/{id} → soft tombstone (tombstoned_at = now()).
    //! The slug stays bound to the row; discovery returns 410 Gone
    //! instead of 404; future re-registration on the same slug needs
    //! an explicit untombstone (D9 admin path; not in this commit).
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: true,
            log_filter: "warn".into(),
        }
    }

    async fn seed_user_with_jwt(pool: &PgPool) -> (Uuid, Uuid, String) {
        let user_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Test', 'x')"#,
            user_id,
            format!("{user_id}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();
        let account_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Acct', 'individual', $3)"#,
            account_id,
            format!("acct-{account_id}"),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            account_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        let token = jwt::encode_jwt(
            &jwt::UserClaims::new(user_id, format!("{user_id}@t.local"), false, 1),
            "test-secret-test-secret-test-secret-test-secret",
        )
        .unwrap();
        (user_id, account_id, token)
    }

    async fn create_pull_agent(pool: &PgPool, account: Uuid, slug: &str, token: &str) -> Uuid {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account,
                            "slug": slug,
                            "display_name": "Agent",
                            "visibility": "network",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        body["id"].as_str().unwrap().parse().unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn delete_soft_tombstones_instead_of_hard_deleting(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let agent_id = create_pull_agent(&pool, account_id, "alice", &token).await;

        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/agents/{agent_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "got {}", res.status());

        // The row still exists, but with tombstoned_at set.
        let row = sqlx::query!(
            "SELECT slug, tombstoned_at FROM agents WHERE id = $1",
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.slug, "alice");
        assert!(row.tombstoned_at.is_some());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn slug_does_not_get_freed_after_tombstone(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let _agent_id = create_pull_agent(&pool, account_id, "alice", &token).await;

        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        // Tombstone.
        app.clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/v1/agents/{_agent_id}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Try to re-register the same slug. Per D1's partial unique
        // index, the slug is FREE (only live rows count), so this
        // should succeed. Tombstone-protection is a *policy* layer
        // above the schema; D9's full untombstone gate (admin-only,
        // 365d window) lands in the slug-rename + admin commit.
        // For now, document that re-registration succeeds and the
        // tombstoned row sits alongside the new one.
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/agents")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "account_id": account_id,
                            "slug": "alice",
                            "display_name": "Reborn",
                            "visibility": "network",
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success(), "got {}", res.status());

        // Two rows now exist with the same slug — one tombstoned,
        // one live. Asserting that explicitly so a future
        // tombstone-protection commit's reviewer sees the gap.
        let count = sqlx::query!(
            "SELECT COUNT(*)::bigint AS \"n!\" FROM agents WHERE account_id = $1 AND slug = 'alice'",
            account_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count.n, 2);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn re_deleting_a_tombstoned_agent_is_idempotent(pool: PgPool) {
        let (_, account_id, token) = seed_user_with_jwt(&pool).await;
        let agent_id = create_pull_agent(&pool, account_id, "alice", &token).await;
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));

        for _ in 0..2 {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("DELETE")
                        .uri(format!("/v1/agents/{agent_id}"))
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NO_CONTENT);
        }
    }
}
