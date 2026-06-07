//! General usage + audit event recording.
//!
//! `usage_events` meters EVERY request (REST routes + each MCP tool),
//! including read-only GETs/pulls — the substrate for future billing.
//! `audit_events` records every WRITE (create/update/delete/state change);
//! read-only pulls are intentionally not audited.
//!
//! Both recorders are best-effort: a failure to write an event must never
//! fail the user's request, so errors are logged and swallowed.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthUser;

/// Record one metered request. `actor` is None for unauthenticated hits
/// (e.g. public discovery). `account_id` is optional — usage is primarily
/// attributed by user/key, and the owning account isn't always known at
/// the middleware layer.
#[allow(clippy::too_many_arguments)]
pub async fn record_usage(
    db: &PgPool,
    actor: Option<&AuthUser>,
    account_id: Option<Uuid>,
    surface: &str,
    action: &str,
    method: &str,
    route: &str,
    status_code: i32,
) {
    let (uid, api_key_id, jti) = match actor {
        Some(a) => (Some(a.user_id), a.api_key_id, a.minted_jti),
        None => (None, None, None),
    };
    let ok = (200..400).contains(&status_code);
    let res = sqlx::query!(
        r#"
        INSERT INTO usage_events
            (id, actor_user_id, account_id, api_key_id, minted_jti,
             surface, action, method, route, status_code, ok)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        Uuid::now_v7(),
        uid,
        account_id,
        api_key_id,
        jti,
        surface,
        action,
        method,
        route,
        status_code,
        ok,
    )
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("usage_events insert failed: {e}");
    }
}

/// Record one write to the audit trail. Only called from write paths
/// (create/update/delete/state change) — never from reads.
#[allow(clippy::too_many_arguments)]
pub async fn record_audit(
    db: &PgPool,
    actor: &AuthUser,
    account_id: Option<Uuid>,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    target_id: Option<Uuid>,
    summary: &str,
    metadata: Value,
) {
    let res = sqlx::query!(
        r#"
        INSERT INTO audit_events
            (id, actor_user_id, account_id, api_key_id, minted_jti,
             action, resource_type, resource_id, target_id, summary, metadata)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        Uuid::now_v7(),
        actor.user_id,
        account_id,
        actor.api_key_id,
        actor.minted_jti,
        action,
        resource_type,
        resource_id,
        target_id,
        summary,
        metadata,
    )
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("audit_events insert failed: {e}");
    }
}
