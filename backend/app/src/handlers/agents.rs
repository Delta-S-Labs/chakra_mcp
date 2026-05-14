//! Agent re-parenting — `POST /v1/agents/{id}/move-to-personal`.
//!
//! The user wants to "unregister an agent from this org", but we don't
//! want to destroy the agent itself: friendships, grants, and the
//! invocation audit trail are all keyed on `agent_id`, and dropping the
//! row would either nuke the history or leave the world half-detached.
//!
//! Instead we re-parent the agent to the caller's personal account.
//! Same agent_id, same capabilities, same everything else — the row's
//! `account_id` and (if there's a slug collision in the destination)
//! `slug` change, and that's it.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct MoveToPersonalResponse {
    pub agent_id: Uuid,
    pub old_account_slug: String,
    pub new_account_slug: String,
    pub new_agent_slug: String,
}

// ─────────────────────────────────────────────────────────
// POST /v1/agents/{id}/move-to-personal
// ─────────────────────────────────────────────────────────
pub async fn move_to_personal(
    State(state): State<AppState>,
    user: AuthUser,
    Path(agent_id): Path<Uuid>,
) -> ApiResult<Json<MoveToPersonalResponse>> {
    let mut tx = state.db.begin().await?;

    // Load the agent and lock the row.
    let agent = sqlx::query!(
        r#"
        SELECT a.id, a.slug, a.account_id, acc.slug AS account_slug
        FROM agents a
        JOIN accounts acc ON acc.id = a.account_id
        WHERE a.id = $1 AND a.tombstoned_at IS NULL
        FOR UPDATE OF a
        "#,
        agent_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Caller must be a member of the current account.
    let is_member = sqlx::query!(
        r#"
        SELECT 1 AS one FROM account_memberships
        WHERE user_id = $1 AND account_id = $2
        LIMIT 1
        "#,
        user.user_id,
        agent.account_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !is_member {
        return Err(ApiError::Forbidden);
    }

    // Resolve the caller's personal account.
    let personal = sqlx::query!(
        r#"
        SELECT id, slug FROM accounts
        WHERE owner_user_id = $1
          AND account_type = 'individual'
          AND tombstoned_at IS NULL
        ORDER BY created_at ASC
        LIMIT 1
        "#,
        user.user_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        ApiError::Conflict("caller has no personal account to move the agent into".into())
    })?;

    // No-op if already there. Return success without modifying anything.
    if personal.id == agent.account_id {
        tx.commit().await?;
        return Ok(Json(MoveToPersonalResponse {
            agent_id: agent.id,
            old_account_slug: agent.account_slug.clone(),
            new_account_slug: personal.slug,
            new_agent_slug: agent.slug,
        }));
    }

    // Allocate a free slug in the destination personal account.
    let new_slug = allocate_free_slug(&mut tx, personal.id, &agent.slug).await?;

    sqlx::query!(
        r#"
        UPDATE agents
        SET account_id = $2, slug = $3
        WHERE id = $1
        "#,
        agent.id,
        personal.id,
        new_slug,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Json(MoveToPersonalResponse {
        agent_id: agent.id,
        old_account_slug: agent.account_slug,
        new_account_slug: personal.slug,
        new_agent_slug: new_slug,
    }))
}

/// Find a slug that doesn't collide in the destination account. Starts
/// with `base`; on collision tries `base-1`, `base-2`, … up to 999.
///
/// Honours the partial unique index on (account_id, slug) WHERE
/// tombstoned_at IS NULL — tombstoned rows don't block.
pub(crate) async fn allocate_free_slug<'c>(
    tx: &mut sqlx::Transaction<'c, sqlx::Postgres>,
    account_id: Uuid,
    base: &str,
) -> Result<String, ApiError> {
    let mut candidate = base.to_string();
    for n in 0..1000 {
        if n > 0 {
            candidate = format!("{base}-{n}");
        }
        let exists = sqlx::query!(
            r#"
            SELECT 1 AS one FROM agents
            WHERE account_id = $1 AND slug = $2 AND tombstoned_at IS NULL
            LIMIT 1
            "#,
            account_id,
            candidate,
        )
        .fetch_optional(&mut **tx)
        .await?
        .is_some();
        if !exists {
            return Ok(candidate);
        }
    }
    Err(ApiError::Conflict(format!(
        "could not find a free slug under base '{base}' after 1000 attempts"
    )))
}

#[cfg(test)]
mod tests {
    //! End-to-end tests for /v1/agents/{id}/move-to-personal and
    //! the cascading DELETE /v1/orgs/{slug}.

    use crate::tests_support::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    #[sqlx::test(migrations = "../migrations")]
    async fn move_to_personal_happy_path(pool: PgPool) {
        let (user, token, personal_account) = seed_user_with_personal(&pool, "alice").await;
        let org_account = seed_org(&pool, "acme-corp", user.user_id).await;
        let agent_id = seed_agent(&pool, org_account, "scheduler", user.user_id).await;

        let state = crate::AppState::new(pool.clone(), test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{agent_id}/move-to-personal"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["agent_id"], serde_json::json!(agent_id));
        assert_eq!(json["old_account_slug"], "acme-corp");
        assert_eq!(json["new_agent_slug"], "scheduler");

        // Database side-effect: the agent's account_id is the personal one now.
        let row = sqlx::query!(
            r#"SELECT account_id, slug FROM agents WHERE id = $1"#,
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.account_id, personal_account);
        assert_eq!(row.slug, "scheduler");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn move_to_personal_slug_collision_gets_suffix(pool: PgPool) {
        let (user, token, personal_account) = seed_user_with_personal(&pool, "alice").await;
        let org_account = seed_org(&pool, "acme-corp", user.user_id).await;

        // Pre-seed a `scheduler` agent inside the personal account so
        // the move from the org collides.
        seed_agent(&pool, personal_account, "scheduler", user.user_id).await;
        let agent_id = seed_agent(&pool, org_account, "scheduler", user.user_id).await;

        let state = crate::AppState::new(pool.clone(), test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{agent_id}/move-to-personal"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // First collision → suffix '-1'.
        assert_eq!(json["new_agent_slug"], "scheduler-1");

        let row = sqlx::query!(
            r#"SELECT account_id, slug FROM agents WHERE id = $1"#,
            agent_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.account_id, personal_account);
        assert_eq!(row.slug, "scheduler-1");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn delete_org_cascades_agents_to_owner_personal(pool: PgPool) {
        let (user, token, personal_account) = seed_user_with_personal(&pool, "alice").await;
        let org_account = seed_org(&pool, "acme-corp", user.user_id).await;
        let a1 = seed_agent(&pool, org_account, "scheduler", user.user_id).await;
        let a2 = seed_agent(&pool, org_account, "billing", user.user_id).await;

        let state = crate::AppState::new(pool.clone(), test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/orgs/acme-corp")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        // The org row is gone.
        let org_count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "c!" FROM accounts WHERE id = $1"#,
            org_account,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(org_count, 0);

        // Both agents are now under the personal account.
        for a in [a1, a2] {
            let row = sqlx::query!(r#"SELECT account_id FROM agents WHERE id = $1"#, a)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(row.account_id, personal_account);
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn delete_org_rejects_non_owner(pool: PgPool) {
        let (owner, _owner_token, _) = seed_user_with_personal(&pool, "alice").await;
        let (_intruder, intruder_token, _) = seed_user_with_personal(&pool, "mallory").await;
        let _org = seed_org(&pool, "acme-corp", owner.user_id).await;

        let state = crate::AppState::new(pool, test_config());
        let res = crate::router(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v1/orgs/acme-corp")
                    .header(header::AUTHORIZATION, format!("Bearer {intruder_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Mallory has no membership, so we 404 (we don't leak existence).
        assert!(
            matches!(res.status(), StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
            "expected 403/404 for non-owner delete, got {}",
            res.status()
        );
    }
}
