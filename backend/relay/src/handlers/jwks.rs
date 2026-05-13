//! `/.well-known/jwks.json` — public keys for verifying Agent Card
//! signatures.
//!
//! Generic A2A clients fetch this endpoint to obtain our Ed25519
//! public keys, then verify the JWS signature on any Agent Card we
//! published. Keys are returned in the JOSE JWKS format (RFC 7517);
//! key entries follow the OKP profile (RFC 8037) for Ed25519.
//!
//! Caching: this endpoint is heavily cached at the edge. Public keys
//! change only on rotation (every 90 days), and we keep retired keys
//! in JWKS during a 30-day overlap so cards signed under a retired
//! key still verify. Cache-Control balances freshness against load:
//! 5-minute browser cache, 1-hour shared/CDN cache.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::agent_card::keys::KeyStore;
use crate::state::RelayState;

/// `GET /.well-known/jwks.json`
pub async fn get_jwks(State(state): State<RelayState>) -> Response {
    let store = KeyStore::new(state.db.clone());
    match store.jwks().await {
        Ok(jwks) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "public, max-age=300, s-maxage=3600"),
            ],
            Json(jwks),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to load JWKS");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                Json(serde_json::json!({"error": "jwks unavailable"})),
            )
                .into_response()
        }
    }
}
