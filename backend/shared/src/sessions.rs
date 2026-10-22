//! Telling a person's own web session apart from credentials that act for
//! them.
//!
//! A user JWT is either the web app's sign-in session or a token minted for
//! an OAuth client or a device pairing (MCP clients, the CLI, agents). Both
//! services refuse the delegated kind where only the person may act: the
//! app's `InteractiveUser` and `AdminUser`, and the relay's `InteractiveUser`
//! (owner consent approvals).

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

/// Whether this JWT was minted for an OAuth client or a device pairing.
pub async fn is_delegated_token(db: &PgPool, jti: Uuid) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar!(
        r#"
        SELECT EXISTS (SELECT 1 FROM oauth_authorizations WHERE minted_jti = $1)
            OR EXISTS (SELECT 1 FROM oauth_device_codes WHERE minted_jti = $1)
            AS "delegated!"
        "#,
        jti,
    )
    .fetch_one(db)
    .await?)
}
