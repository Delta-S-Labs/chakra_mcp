//! Unified "paired sessions" view across three credential pathways:
//!   * device_flow — `oauth_device_codes` row, approved + token minted
//!   * oauth       — `oauth_authorizations` row that got token-minted
//!   * api_key     — `api_keys` row
//!
//! From a user's perspective these are all "things that can talk to my
//! account on my behalf"; this module is the single endpoint the
//! frontend hits to render the unified sessions list. Per-source
//! semantics are spelled out below in `list`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingKind {
    DeviceFlow,
    Oauth,
    ApiKey,
}

#[derive(Debug, Serialize)]
pub struct PairingDto {
    pub kind: PairingKind,
    pub id: Uuid,
    pub agent_id: Option<Uuid>,
    pub agent_slug: Option<String>,
    /// Short label for the UI — for api_key this is the user-chosen
    /// `name`; for device_flow this is the agent's display name (or
    /// the slug hint) if available; for oauth it's the client_name.
    pub label: Option<String>,
    pub paired_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

// ─────────────────────────────────────────────────────────
// GET /v1/pairings
//
// Surfaces every pairing the caller owns. We do this as three small
// queries + an in-memory merge rather than a UNION ALL in SQL —
// sqlx::query! generates a distinct anonymous row struct per query,
// and dragging three column shapes through one CTE just to share an
// ORDER BY isn't worth the readability cost.
// ─────────────────────────────────────────────────────────
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<PairingDto>>> {
    let mut out: Vec<PairingDto> = Vec::new();

    // ── Device flow ─────────────────────────────────────
    // A row counts as a pairing iff the human actually approved it.
    // Approval implies an agent_id was bound; we join through it to
    // surface the slug + display name in the response.
    let device_rows = sqlx::query!(
        r#"
        SELECT
            d.id,
            d.approved_at,
            d.consumed_at,
            d.revoked_at,
            d.approved_agent_id,
            a.slug AS "agent_slug?",
            a.display_name AS "agent_display_name?"
        FROM oauth_device_codes d
        LEFT JOIN agents a ON a.id = d.approved_agent_id
        WHERE d.approved_user_id = $1
          AND d.approved_at IS NOT NULL
        ORDER BY d.approved_at DESC
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    for r in device_rows {
        out.push(PairingDto {
            kind: PairingKind::DeviceFlow,
            id: r.id,
            agent_id: r.approved_agent_id,
            agent_slug: r.agent_slug.clone(),
            label: r.agent_display_name.or(r.agent_slug),
            // `approved_at` is non-null by the WHERE clause above; the
            // sqlx macro types it as Option<_> regardless.
            paired_at: r.approved_at.expect("WHERE approved_at IS NOT NULL"),
            last_used_at: r.consumed_at,
            revoked_at: r.revoked_at,
        });
    }

    // ── OAuth code grant ─────────────────────────────────
    // Each row represents a single token mint (used_at != NULL). We
    // can't tie these to an agent today — auth-code flow is for MCP
    // hosts, not registered agents. The frontend renders these as
    // "External client: <client_name>".
    let oauth_rows = sqlx::query!(
        r#"
        SELECT
            o.id,
            o.used_at AS "used_at!",
            o.revoked_at,
            c.client_name
        FROM oauth_authorizations o
        JOIN oauth_clients c ON c.client_id = o.client_id
        WHERE o.user_id = $1
          AND o.used_at IS NOT NULL
        ORDER BY o.used_at DESC
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    for r in oauth_rows {
        out.push(PairingDto {
            kind: PairingKind::Oauth,
            id: r.id,
            agent_id: None,
            agent_slug: None,
            label: Some(r.client_name),
            paired_at: r.used_at,
            last_used_at: None,
            revoked_at: r.revoked_at,
        });
    }

    // ── API keys ─────────────────────────────────────────
    let api_rows = sqlx::query!(
        r#"
        SELECT id, name, last_used_at, revoked_at, created_at
        FROM api_keys
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;
    for r in api_rows {
        out.push(PairingDto {
            kind: PairingKind::ApiKey,
            id: r.id,
            agent_id: None,
            agent_slug: None,
            label: Some(r.name),
            paired_at: r.created_at,
            last_used_at: r.last_used_at,
            revoked_at: r.revoked_at,
        });
    }

    // Newest-first across all three sources.
    out.sort_by_key(|p| std::cmp::Reverse(p.paired_at));

    Ok(Json(out))
}

// ─────────────────────────────────────────────────────────
// POST /v1/pairings/{kind}/{id}/revoke
//
// Sets `revoked_at = now()` on the matching row and, where possible,
// also kills the JWT we minted from it via the `revoked_tokens`
// table. The minted-jti link was added in migration 0017; rows
// created before that migration have `minted_jti IS NULL`, in which
// case the JWT survives until natural expiry — we accept that, since
// the device-code / auth-code row is itself revoked and can't issue
// another token.
// ─────────────────────────────────────────────────────────
pub async fn revoke(
    State(state): State<AppState>,
    user: AuthUser,
    Path((kind, id)): Path<(String, Uuid)>,
) -> ApiResult<StatusCode> {
    match kind.as_str() {
        "device_flow" => revoke_device_flow(&state, user.user_id, id).await,
        "oauth" => revoke_oauth(&state, user.user_id, id).await,
        "api_key" => revoke_api_key(&state, user.user_id, id).await,
        _ => Err(ApiError::InvalidRequest(format!(
            "unknown pairing kind '{kind}'; expected one of device_flow, oauth, api_key"
        ))),
    }
}

async fn revoke_device_flow(state: &AppState, user_id: Uuid, id: Uuid) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;

    let row = sqlx::query!(
        r#"
        SELECT id, approved_user_id, revoked_at, minted_jti, expires_at
        FROM oauth_device_codes
        WHERE id = $1
        FOR UPDATE
        "#,
        id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.approved_user_id != Some(user_id) {
        // Either the row was never approved, or it's not the caller's.
        // Same response either way — we don't leak existence.
        return Err(ApiError::NotFound);
    }

    if row.revoked_at.is_none() {
        sqlx::query!(
            r#"UPDATE oauth_device_codes SET revoked_at = now() WHERE id = $1"#,
            row.id,
        )
        .execute(&mut *tx)
        .await?;
    }

    // Best-effort JWT kill: if we recorded the minted jti, drop it
    // into revoked_tokens. We don't know the original `exp` either,
    // so we use the device code's `expires_at` as an upper bound for
    // the GC sweep. That's a *much* longer ceiling than the JWT's
    // real exp (24h vs ~10min for the device-code row), but the
    // sweep only deletes rows whose expires_at has passed, so it's
    // always safe to overestimate here.
    if let Some(jti) = row.minted_jti {
        // The JWT's real exp is `approved_at + 24h`. We don't store
        // approved_at past consumption, so the conservative call is
        // `now() + 24h`.
        let exp_ceiling = chrono::Utc::now() + chrono::Duration::hours(25);
        sqlx::query!(
            r#"
            INSERT INTO revoked_tokens (jti, user_id, expires_at, reason)
            VALUES ($1, $2, $3, 'pairing_revoked')
            ON CONFLICT (jti) DO NOTHING
            "#,
            jti,
            user_id,
            exp_ceiling,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_oauth(state: &AppState, user_id: Uuid, id: Uuid) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;

    let row = sqlx::query!(
        r#"
        SELECT id, user_id, revoked_at, minted_jti
        FROM oauth_authorizations
        WHERE id = $1
        FOR UPDATE
        "#,
        id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    if row.user_id != user_id {
        return Err(ApiError::NotFound);
    }

    if row.revoked_at.is_none() {
        sqlx::query!(
            r#"UPDATE oauth_authorizations SET revoked_at = now() WHERE id = $1"#,
            row.id,
        )
        .execute(&mut *tx)
        .await?;
    }

    if let Some(jti) = row.minted_jti {
        let exp_ceiling = chrono::Utc::now() + chrono::Duration::hours(25);
        sqlx::query!(
            r#"
            INSERT INTO revoked_tokens (jti, user_id, expires_at, reason)
            VALUES ($1, $2, $3, 'pairing_revoked')
            ON CONFLICT (jti) DO NOTHING
            "#,
            jti,
            user_id,
            exp_ceiling,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_api_key(state: &AppState, user_id: Uuid, id: Uuid) -> ApiResult<StatusCode> {
    // Same code path as `api_keys::revoke`. We inline it here rather
    // than reuse the handler so the kind dispatch above stays
    // symmetric across all three branches.
    let result = sqlx::query!(
        r#"
        UPDATE api_keys
        SET revoked_at = now()
        WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL
        "#,
        id,
        user_id,
    )
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        // Either no such key, not the caller's, or already revoked —
        // collapse to 404 for symmetry with the other branches and to
        // match the existing `api_keys::revoke` shape.
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    //! Pairings list + revoke. Covers all three pathways:
    //!   * device_flow approval surfaces with the agent linked
    //!   * api key surfaces with its name as the label
    //!   * revoke flips `revoked_at` and (for api_key) further auth
    //!     with that credential fails

    use crate::tests_support::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    #[sqlx::test(migrations = "../migrations")]
    async fn list_surfaces_device_flow_and_api_key(pool: PgPool) {
        let (user, token, personal_account) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, personal_account, "hermes", user.user_id).await;
        seed_approved_device_flow(&pool, user.user_id, agent_id).await;
        let (key_id, _plaintext) = seed_api_key(&pool, user.user_id, "ci key").await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/pairings")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            rows.len(),
            2,
            "expected device_flow + api_key rows: {rows:?}"
        );

        // Find each kind. Order is by paired_at DESC, but the helper
        // seeds them within the same millisecond — sort kinds for a
        // stable assertion.
        let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"device_flow"));
        assert!(kinds.contains(&"api_key"));

        // The device-flow row should carry the agent_id we approved.
        let device = rows
            .iter()
            .find(|r| r["kind"] == "device_flow")
            .expect("device_flow row");
        assert_eq!(device["agent_id"], serde_json::json!(agent_id));
        assert_eq!(device["agent_slug"], "hermes");

        // The api_key row should carry the user-chosen name.
        let key = rows
            .iter()
            .find(|r| r["kind"] == "api_key")
            .expect("api_key row");
        assert_eq!(key["id"], serde_json::json!(key_id));
        assert_eq!(key["label"], "ci key");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn revoke_api_key_kills_future_requests(pool: PgPool) {
        let (user, jwt, _personal) = seed_user_with_personal(&pool, "alice").await;
        let (key_id, plaintext) = seed_api_key(&pool, user.user_id, "ci key").await;

        let state = crate::AppState::new(pool.clone(), test_config());

        // Sanity: the API-key auth path works pre-revoke.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {plaintext}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "API key should authenticate");

        // Revoke through the unified path.
        let res = crate::router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/pairings/api_key/{key_id}/revoke"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // The plaintext API key no longer authenticates.
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/me")
                    .header(header::AUTHORIZATION, format!("Bearer {plaintext}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "revoked API key must not authenticate"
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn revoke_device_flow_sets_revoked_at(pool: PgPool) {
        let (user, jwt, personal) = seed_user_with_personal(&pool, "alice").await;
        let agent_id = seed_agent(&pool, personal, "hermes", user.user_id).await;
        let device_id = seed_approved_device_flow(&pool, user.user_id, agent_id).await;

        let state = crate::AppState::new(pool.clone(), test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/pairings/device_flow/{device_id}/revoke"))
                    .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        let row = sqlx::query!(
            r#"SELECT revoked_at FROM oauth_device_codes WHERE id = $1"#,
            device_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.revoked_at.is_some(), "revoked_at must be set");
    }
}
