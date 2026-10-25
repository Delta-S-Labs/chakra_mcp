//! Deleting a call's content (spec §9): when its TTL runs out (the worker),
//! or at once when an owner asks.
//!
//! The content goes: every entry's text, data and tags; the previews and
//! errors of the call's invocations; the topic and the decline reason. A
//! stub stays: the call row, each entry's kind, sender, size, `blocked` and
//! safety findings (which never hold a removed value).

use chakramcp_shared::error::ApiError;
use sqlx::PgConnection;
use uuid::Uuid;

/// The error an invocation closed by a purge is left with: the one error
/// the purge keeps, since it isn't content.
pub const CLOSED: &str = "call content deleted";

/// Close the call's unfinished invocations: their input is what an agent
/// receives, so they can't keep running without it.
pub async fn close_invocations(conn: &mut PgConnection, call_id: Uuid) -> Result<u64, ApiError> {
    Ok(crate::consent::cascade::cancel_for_call(conn, call_id, CLOSED).await?)
}

/// Delete the call's content. Undelivered entries that carried content are
/// dropped from the feed; events (`ended` and the like) are still delivered.
pub async fn purge_call(conn: &mut PgConnection, call_id: Uuid) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE call_entries SET text = NULL, data = NULL, tags = NULL WHERE call_id = $1",
        call_id,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        r#"
        UPDATE relay_invocations
           SET input_preview = NULL, output_preview = NULL,
               error_message = CASE WHEN error_message = $2 THEN error_message END
         WHERE call_id = $1
        "#,
        call_id,
        CLOSED,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        r#"
        DELETE FROM call_deliveries d
         USING call_entries e
         WHERE d.entry_id = e.id AND e.call_id = $1
           AND d.delivered_at IS NULL AND e.kind IN ('message', 'declined')
        "#,
        call_id,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        r#"
        UPDATE calls SET topic = NULL, decline_reason = NULL, purged_at = now()
         WHERE id = $1
        "#,
        call_id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
