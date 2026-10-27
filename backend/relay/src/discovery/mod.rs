//! The agent query layer (MCP and discovery spec §7.2): ranked search, the
//! fields every agent list shares, profiles and the caller's standing. Used
//! by REST discovery (`handlers::discovery`), the network list, featured
//! agents and, from D3, the MCP tools.

pub mod fields;
pub mod profile;
pub mod search;
pub mod standing;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderName};
use chakramcp_shared::error::ApiError;

use crate::auth::AuthUser;
use crate::state::RelayState;

/// The caller, if signed in. No `Authorization` header means signed out. A
/// header that doesn't authenticate is a 401 (with the usual challenge), so
/// a client notices it lost its session rather than silently losing its
/// standing (spec §5.5).
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<AuthUser>);

impl FromRequestParts<RelayState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &RelayState,
    ) -> Result<Self, Self::Rejection> {
        match parts.headers.get(header::AUTHORIZATION) {
            None => Ok(Self(None)),
            Some(value) => crate::auth::authenticate(state, value.to_str().ok())
                .await
                .map(|user| Self(Some(user)))
                .ok_or(ApiError::Unauthorized),
        }
    }
}

/// `Cache-Control` and `Vary` for a discovery response: shared caches may
/// keep what a signed-out caller sees, never what a signed-in one does.
pub fn cache_headers(signed_in: bool, public: &'static str) -> [(HeaderName, &'static str); 2] {
    [
        (
            header::CACHE_CONTROL,
            if signed_in {
                "private, no-store"
            } else {
                public
            },
        ),
        (header::VARY, "Authorization"),
    ]
}

/// How a viewer relates to the account that owns an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Relation {
    /// A member of the owning account.
    pub member: bool,
    /// Shares an organization with a member of the owning account
    /// (`auth::shares_org_with_account`).
    pub shares_org: bool,
}

impl Relation {
    /// Whether an agent or capability with this `visibility` shows to the
    /// viewer: `network` to everyone, `org` to org-sharers and members,
    /// `private` to members.
    pub fn sees(self, visibility: &str) -> bool {
        match visibility {
            "network" => true,
            "org" => self.member || self.shares_org,
            _ => self.member,
        }
    }
}

#[cfg(test)]
mod tests;
