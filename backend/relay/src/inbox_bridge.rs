//! Pull-mode inbox bridge — park (D5c).
//!
//! When the policy gate authorizes an A2A call to a *pull*-mode
//! target (an agent that has no public host and runs `inbox.serve()`
//! against our relay), this module:
//!
//! 1. Parses the inbound JSON-RPC envelope just enough to recover
//!    the request `id` (so we can echo it on the response).
//! 2. Inserts a `relay_invocations` row with `status='pending'` and
//!    the parsed A2A body in `input_preview`. The existing
//!    `/v1/inbox` polling endpoint (migration 0007) picks this up
//!    automatically — no schema migration needed.
//! 3. Builds an A2A `Task` payload with `state: "submitted"` and
//!    `id` = the invocation UUID. The caller's `GetTask` polling
//!    (D5d) returns the same `id` once the granter posts a result.
//!
//! Why this shape:
//!
//! - The legacy `/v1/inbox` + `/v1/invocations/{id}/result` endpoints
//!   were preserved on `relay.chakramcp.com` for SDK back-compat
//!   (per discovery spec Override #2). Reusing them here means a
//!   pull-mode agent SDK that polled the inbox before the migration
//!   continues to work unchanged — its handler still sees an
//!   `Invocation` dict with `input_preview`, posts back to
//!   `/v1/invocations/{id}/result` with `output`, and the relay
//!   translates that back to an A2A `Task.completed` for the
//!   caller. Friendship/grant context surfaces alongside (added
//!   earlier in the migration).
//! - D5d's GetTask handler (next) wraps the same row.

use axum::body::Bytes;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::policy::Authorized;

#[derive(Debug, thiserror::Error)]
pub enum ParkError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("inbound body is too large to park (limit 16 KB; got {0})")]
    BodyTooLarge(usize),
}

/// Result of a successful park: the task id (= invocation row id),
/// the JSON-RPC request id we'll echo on the response, and a
/// pre-built A2A Task payload ready to ship as `result`.
#[derive(Debug, Clone)]
pub struct Parked {
    pub task_id: Uuid,
    pub jsonrpc_id: serde_json::Value,
    pub task: A2aTask,
}

/// Minimal A2A v0.3 Task subset — the shape `SendMessage` returns
/// when the response is asynchronous. Just enough fields for the
/// caller's GetTask poll loop to start. D5d returns the same shape
/// with the state filled in based on the invocation's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct A2aTask {
    pub id: String,
    pub status: A2aTaskStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct A2aTaskStatus {
    pub state: String, // "submitted" | "working" | "completed" | "failed" | ...
}

const PREVIEW_BYTES: usize = 16 * 1024;

/// Park an authorized pull-mode A2A call.
pub async fn park(
    db: &PgPool,
    authz: &Authorized,
    capability_name: &str,
    request_body: Bytes,
) -> Result<Parked, ParkError> {
    if request_body.len() > PREVIEW_BYTES {
        return Err(ParkError::BodyTooLarge(request_body.len()));
    }

    // Parse what we can. Failure is OK — we still park, with a marker.
    let parsed: serde_json::Value =
        serde_json::from_slice(&request_body).unwrap_or(serde_json::json!({
            "_chk_unparseable": true,
            "_chk_byte_count": request_body.len(),
        }));

    // Extract the JSON-RPC request id (so we echo it on the response)
    // and the SendMessage params (which become input_preview for the
    // granter's `inbox.serve()` handler).
    let jsonrpc_id = parsed
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let params = parsed
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let task_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO relay_invocations
            (id, grant_id, granter_agent_id, grantee_agent_id, capability_id,
             capability_name, invoked_by_user_id, status, elapsed_ms, input_preview)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 0, $8)
        "#,
        task_id,
        authz.grant_id,
        authz.target_agent_id,
        authz.caller_agent_id,
        authz.capability_id,
        capability_name,
        authz.caller_user_id,
        params,
    )
    .execute(db)
    .await?;

    Ok(Parked {
        task_id,
        jsonrpc_id,
        task: A2aTask {
            id: task_id.to_string(),
            status: A2aTaskStatus {
                state: "submitted".to_string(),
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Authorized;

    fn sample_authz_for(
        caller_user: Uuid,
        caller_account: Uuid,
        caller_agent: Uuid,
        target_account: Uuid,
        target_agent: Uuid,
        capability: Uuid,
        grant: Uuid,
    ) -> Authorized {
        Authorized {
            caller_user_id: caller_user,
            caller_account_id: caller_account,
            caller_agent_id: caller_agent,
            target_account_id: target_account,
            target_agent_id: target_agent,
            capability_id: capability,
            grant_id: grant,
            target_is_push: false,
        }
    }

    /// Insert minimal rows the FKs need. Returns the seven UUIDs
    /// the Authorized struct wires together.
    async fn seed_for_park(
        pool: &PgPool,
    ) -> (Uuid, Uuid, Uuid, Uuid, Uuid, Uuid, Uuid) {
        let user = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'U', 'x')"#,
            user,
            format!("u-{user}@t.local"),
        )
        .execute(pool)
        .await
        .unwrap();

        let caller_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, 'Caller', 'individual', $3)"#,
            caller_account,
            format!("ca-{caller_account}"),
            user,
        )
        .execute(pool)
        .await
        .unwrap();
        let caller_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name)
               VALUES ($1, $2, 'caller', 'C')"#,
            caller_agent,
            caller_account,
        )
        .execute(pool)
        .await
        .unwrap();

        let target_account = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Target', 'individual')"#,
            target_account,
            format!("ta-{target_account}"),
        )
        .execute(pool)
        .await
        .unwrap();
        let target_agent = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name)
               VALUES ($1, $2, 'target', 'T')"#,
            target_agent,
            target_account,
        )
        .execute(pool)
        .await
        .unwrap();
        let capability = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema)
               VALUES ($1, $2, 'do', 'Do.', '{}'::jsonb, '{}'::jsonb)"#,
            capability,
            target_agent,
        )
        .execute(pool)
        .await
        .unwrap();
        let grant = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO grants
                  (id, granter_agent_id, grantee_agent_id, capability_id, status)
               VALUES ($1, $2, $3, $4, 'active')"#,
            grant,
            target_agent,
            caller_agent,
            capability,
        )
        .execute(pool)
        .await
        .unwrap();
        (user, caller_account, caller_agent, target_account, target_agent, capability, grant)
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn park_inserts_pending_row_and_returns_task(pool: PgPool) {
        let (u, ca, cg, ta, tg, c, g) = seed_for_park(&pool).await;
        let authz = sample_authz_for(u, ca, cg, ta, tg, c, g);
        let body = Bytes::from_static(
            br#"{"jsonrpc":"2.0","id":42,"method":"SendMessage","params":{"message":"hi"}}"#,
        );
        let parked = park(&pool, &authz, "do", body).await.unwrap();

        assert_eq!(parked.jsonrpc_id, serde_json::json!(42));
        assert_eq!(parked.task.status.state, "submitted");
        assert_eq!(parked.task.id, parked.task_id.to_string());

        // Row landed with status='pending'.
        let row = sqlx::query!(
            r#"SELECT status, capability_name, input_preview, granter_agent_id,
                      grantee_agent_id, capability_id
                 FROM relay_invocations WHERE id = $1"#,
            parked.task_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.status, "pending");
        assert_eq!(row.capability_name, "do");
        assert_eq!(row.granter_agent_id, Some(tg));
        assert_eq!(row.grantee_agent_id, Some(cg));
        assert_eq!(row.capability_id, Some(c));
        // input_preview holds the SendMessage params (NOT the whole envelope)
        // so the granter SDK's handler sees the same shape it always did.
        assert_eq!(
            row.input_preview,
            Some(serde_json::json!({"message": "hi"}))
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn park_handles_unparseable_body(pool: PgPool) {
        let (u, ca, cg, ta, tg, c, g) = seed_for_park(&pool).await;
        let authz = sample_authz_for(u, ca, cg, ta, tg, c, g);
        let body = Bytes::from_static(b"this is not json{{");
        let parked = park(&pool, &authz, "do", body).await.unwrap();
        let row = sqlx::query!(
            "SELECT status, input_preview FROM relay_invocations WHERE id = $1",
            parked.task_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.status, "pending");
        // Falls through to a Null because params didn't exist on the parsed
        // marker object either; the row still parks.
        assert!(row.input_preview.is_some());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn park_rejects_oversize_body(pool: PgPool) {
        let (u, ca, cg, ta, tg, c, g) = seed_for_park(&pool).await;
        let authz = sample_authz_for(u, ca, cg, ta, tg, c, g);
        // 17 KB > 16 KB cap.
        let body = Bytes::from(vec![b'a'; 17 * 1024]);
        let r = park(&pool, &authz, "do", body).await;
        assert!(matches!(r, Err(ParkError::BodyTooLarge(_))));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn echoes_jsonrpc_id_string_form(pool: PgPool) {
        // JSON-RPC ids can be strings, not just numbers.
        let (u, ca, cg, ta, tg, c, g) = seed_for_park(&pool).await;
        let authz = sample_authz_for(u, ca, cg, ta, tg, c, g);
        let body = Bytes::from_static(
            br#"{"jsonrpc":"2.0","id":"req-abc","method":"SendMessage","params":{}}"#,
        );
        let parked = park(&pool, &authz, "do", body).await.unwrap();
        assert_eq!(parked.jsonrpc_id, serde_json::json!("req-abc"));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn missing_id_becomes_null(pool: PgPool) {
        let (u, ca, cg, ta, tg, c, g) = seed_for_park(&pool).await;
        let authz = sample_authz_for(u, ca, cg, ta, tg, c, g);
        let body = Bytes::from_static(
            br#"{"jsonrpc":"2.0","method":"SendMessage","params":{}}"#,
        );
        let parked = park(&pool, &authz, "do", body).await.unwrap();
        assert_eq!(parked.jsonrpc_id, serde_json::Value::Null);
    }
}
