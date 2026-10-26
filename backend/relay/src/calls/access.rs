//! Who may act on a call, and as whom (spec §4).
//!
//! - **Parties** act as one of the call's two agents: a member of the
//!   agent's account whose credential may operate it (the inbox's rule).
//!   Owners don't post into calls, so an interactive web session is refused.
//! - **Owners** end calls and delete their content from the web: an
//!   interactive session that's a member of either party's account.
//! - **Readers** are members of either party's account.

use axum::http::StatusCode;
use chakramcp_shared::error::ApiError;
use uuid::Uuid;

use super::Call;
use crate::auth::{is_interactive, user_is_member, AuthUser};
use crate::state::RelayState;

/// The agent a party route acts as.
#[derive(Debug, Clone, Copy)]
pub struct Acting {
    pub agent_id: Uuid,
    pub account_id: Uuid,
}

/// Act as `agent_id` (a party route): 404 when it doesn't exist, is
/// tombstoned or isn't the caller's; 403 when the credential is scoped to
/// other agents or is a web session.
pub async fn act_as(
    state: &RelayState,
    user: &AuthUser,
    agent_id: Uuid,
) -> Result<Acting, ApiError> {
    let account_id = sqlx::query_scalar!(
        "SELECT account_id FROM agents WHERE id = $1 AND tombstoned_at IS NULL",
        agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !user_is_member(&state.db, user.user_id, account_id).await? {
        return Err(ApiError::NotFound);
    }
    crate::auth::require_scope(state, user, agent_id).await?;
    if is_interactive(state, user).await? {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            "chk.calls.owner_session",
            "owners read and end calls on the web, but don't act in them: \
             use the agent's API key or token",
        ));
    }
    Ok(Acting {
        agent_id,
        account_id,
    })
}

/// An owner route: an interactive web session, a member of either party's
/// account.
pub async fn owner(state: &RelayState, user: &AuthUser, call: &Call) -> Result<(), ApiError> {
    if !is_interactive(state, user).await? {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            "chk.calls.owner_only",
            "only an owner signed in on the web can do this",
        ));
    }
    can_read(state, user, call).await
}

/// A reader: a member of either party's account (404 otherwise, so a call's
/// existence isn't leaked).
pub async fn can_read(state: &RelayState, user: &AuthUser, call: &Call) -> Result<(), ApiError> {
    for account in [call.caller.account_id, call.callee.account_id] {
        if user_is_member(&state.db, user.user_id, account).await? {
            return Ok(());
        }
    }
    Err(ApiError::NotFound)
}
