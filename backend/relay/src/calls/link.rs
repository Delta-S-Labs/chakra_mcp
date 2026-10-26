//! Capabilities in a call (spec §7): `invoke` with a `call_id`.
//!
//! The call is checked where the invocation's two agents are known, before
//! limits, compliance or screening spend anything. The invocation row is
//! created with `call_id` in the same statement, so the purge always finds
//! it; its transcript entry is appended right after, best effort.

use axum::http::StatusCode;
use chakramcp_shared::error::ApiError;
use sqlx::PgPool;
use uuid::Uuid;

use super::{access, append, bad_terms, load_call, not_active, NewEntry};
use crate::auth::AuthUser;
use crate::state::RelayState;

/// Check that `grantee` may invoke `granter`'s capability inside `call_id`:
/// calls are on, the call is active, `grantee` is a party acting under the
/// party rule, and `granter` is the other party.
pub async fn check(
    state: &RelayState,
    user: &AuthUser,
    call_id: Option<Uuid>,
    grantee: Uuid,
    granter: Uuid,
) -> Result<(), ApiError> {
    let Some(call_id) = call_id else {
        return Ok(());
    };
    if !state.calls.enabled {
        return Err(bad_terms("calls aren't enabled on this relay"));
    }
    let call = load_call(&state.db, call_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    access::can_read(state, user, &call).await?;
    access::act_as(state, user, grantee).await?;
    let not_a_party =
        |message: &str| ApiError::coded(StatusCode::FORBIDDEN, "chk.calls.not_a_party", message);
    let Some(role) = call.role_of(grantee) else {
        return Err(not_a_party("the invoking agent isn't on this call"));
    };
    if call.party(role.other()).agent_id != Some(granter) {
        return Err(not_a_party(
            "a call carries requests to the other party's capabilities only",
        ));
    }
    if call.state != "active" {
        return Err(not_active(format!("this call is {}", call.state)));
    }
    Ok(())
}

/// Append the `invocation` entry for an invocation made in `call_id`. Not
/// delivered: the invocation reaches its agent through the inbox. A failure
/// is logged; the invocation stands either way.
pub async fn note(db: &PgPool, call_id: Option<Uuid>, invocation_id: Uuid, from: Uuid) {
    let Some(call_id) = call_id else {
        return;
    };
    let result = async {
        let mut tx = db.begin().await?;
        let mut entry = NewEntry::event("invocation", Some(from));
        entry.invocation_id = Some(invocation_id);
        append(&mut tx, call_id, entry, &[]).await?;
        tx.commit().await?;
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!(%call_id, %invocation_id, error = %e, "couldn't add an invocation to its call");
    }
}
