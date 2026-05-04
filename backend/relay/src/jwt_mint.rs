//! Relay-issued bearer tokens for forwarded A2A calls (D5a).
//!
//! Per discovery spec Override #3: ChakraMCP holds its own Ed25519
//! signing key. On every authorized A2A forward (push mode), the
//! relay mints a short-lived JWT and presents it as
//! `Authorization: Bearer <jwt>` to the target's canonical A2A
//! endpoint. Targets verify against our public key in JWKS at
//! `chakramcp.com/.well-known/jwks.json`.
//!
//! No shared secret with the target. The grant-holder (caller) never
//! sees the bearer either — only the relay knows its private key.
//! That's the whole point: trust is mediated through us, not stored
//! at either end.
//!
//! Wire shape: standard JWT Compact (RFC 7519 over RFC 7515 JWS):
//!     BASE64URL(header) "." BASE64URL(payload) "." BASE64URL(signature)
//!
//! - Header: `{"alg":"EdDSA","kid":"<kid>","typ":"JWT"}`
//! - Payload: `RelayClaims` serialized to JSON.
//! - Signature: Ed25519 over `header.payload` (the "signing input").
//!
//! Re-uses the SAME signing keys as the Agent Card signer (D2b). One
//! `kid` in JWKS covers both card signatures and forwarded-call
//! bearers; rotation costs nothing extra.
//!
//! Threat model + mitigations:
//! - Replay: every JWT carries a unique `jti` (UUID v7) + a 60s
//!   `exp`. Targets MAY (and the docs recommend) cache jti→exp to
//!   reject replays within the window; ChakraMCP-aware targets get
//!   that for free.
//! - Token theft from log → impersonation: bound by 60s `exp`.
//! - Privilege escalation: `sub`, `aud`, `capability`, `grant_id`
//!   are all verified by the SDK in target agents (cross-checks
//!   against the published agent card + the granter's view).
//! - Key compromise: rotation already covered by D2b.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, Signer as _, Verifier as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent_card::signer::{SigningKey, VerifyingKey, ALG_EDDSA};
use crate::policy::Authorized;

/// How long every minted JWT is valid for. Short enough that token
/// theft from logs is bounded; long enough that target-side clock
/// skew + network latency don't reject legitimate calls.
pub const DEFAULT_TTL_SECONDS: i64 = 60;

/// JWT `typ` (RFC 8725 §3.11) — "JWT" so a target can distinguish
/// our forwarded-call bearer from the cards' standalone signatures
/// (which carry `typ` absent).
pub const JWT_TYP: &str = "JWT";

/// `iss` claim — the relay's identity. Targets MAY pin this so they
/// only accept tokens minted by ChakraMCP and not, say, a forked
/// network.
pub const DEFAULT_ISSUER: &str = "https://chakramcp.com";

/// Claims carried in a relay-minted JWT. Target agents (especially
/// our SDKs) deserialize this to extract trust context cheaply
/// instead of re-querying the relay (the same "trust the network"
/// principle as the inbox-side friendship_context / grant_context
/// payload).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelayClaims {
    /// Issuer — always the relay's well-known URL.
    pub iss: String,
    /// Audience — the target agent's UUID. Stops a token minted
    /// for agent A from being replayed against agent B.
    pub aud: String,
    /// Subject — the caller agent's UUID.
    pub sub: String,
    /// Expiration (unix seconds). Hard-clamped to 60s from `iat`.
    pub exp: i64,
    /// Issued-at (unix seconds).
    pub iat: i64,
    /// Token id — unique UUID v7 for replay protection.
    pub jti: String,
    /// Caller's account id. Targets can attribute audit events to
    /// the *organization* without an extra lookup.
    pub caller_account_id: String,
    /// Target's account id (for symmetric audit on the target side).
    pub target_account_id: String,
    /// Capability being invoked.
    pub capability_id: String,
    /// Grant id authorizing this call. Targets can hand this back
    /// to ChakraMCP for cross-checks (revocation status, rate
    /// limits) when paranoid.
    pub grant_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MintError {
    #[error("failed to serialize claims: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VerifyJwtError {
    #[error("token format is not three base64url segments separated by '.'")]
    MalformedToken,
    #[error("header is not valid base64url JSON")]
    MalformedHeader,
    #[error("payload is not valid base64url JSON")]
    MalformedPayload,
    #[error("signature is not 64 base64url-decoded bytes")]
    MalformedSignature,
    #[error("alg in header is not EdDSA")]
    UnsupportedAlg,
    #[error("kid in header doesn't match any published key")]
    UnknownKid,
    #[error("signature verification failed")]
    InvalidSignature,
    #[error("token is expired (exp in the past)")]
    Expired,
    #[error("token is not yet valid (iat in the future beyond clock-skew tolerance)")]
    NotYetValid,
}

/// Mint a JWT for a single forwarded call. The returned string is a
/// JWT Compact-form bearer ready to drop into an `Authorization`
/// header: `Authorization: Bearer <returned>`.
pub fn mint_for_proxied_call(
    authz: &Authorized,
    key: &SigningKey,
    issued_at: DateTime<Utc>,
    ttl_seconds: i64,
) -> Result<String, MintError> {
    let claims = RelayClaims {
        iss: DEFAULT_ISSUER.to_string(),
        aud: authz.target_agent_id.to_string(),
        sub: authz.caller_agent_id.to_string(),
        exp: (issued_at + Duration::seconds(ttl_seconds)).timestamp(),
        iat: issued_at.timestamp(),
        jti: Uuid::now_v7().to_string(),
        caller_account_id: authz.caller_account_id.to_string(),
        target_account_id: authz.target_account_id.to_string(),
        capability_id: authz.capability_id.to_string(),
        grant_id: authz.grant_id.to_string(),
    };
    mint_jwt(&claims, key)
}

/// Lower-level mint: produce a JWT for arbitrary claims. Public so
/// tests + admin tooling can issue tokens without going through
/// `Authorized`.
pub fn mint_jwt(claims: &RelayClaims, key: &SigningKey) -> Result<String, MintError> {
    // Hand-built header. We control the field set; using `format!`
    // here is more robust than serde_json (no field-ordering
    // surprises) and the shape is small + fixed.
    let header_json = format!(
        r#"{{"alg":"{ALG_EDDSA}","kid":"{kid}","typ":"{JWT_TYP}"}}"#,
        kid = key.kid,
    );
    let header_b64 = URL_SAFE_NO_PAD.encode(header_json.as_bytes());

    let payload_bytes = serde_json::to_vec(claims)?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_bytes);

    let signing_input = format!("{header_b64}.{payload_b64}");
    let sig: Signature = key.inner.sign(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig.to_bytes());

    Ok(format!("{signing_input}.{sig_b64}"))
}

/// Verify a relay-minted JWT against a set of public keys (from
/// JWKS). Returns the claims on success.
///
/// `now` is passed in (rather than read internally) so callers can
/// run with a controlled clock — useful for tests and for clock-skew
/// allowances ("accept tokens issued up to 60s in the future").
pub fn decode_relay_jwt(
    token: &str,
    keys: &[VerifyingKey],
    now: DateTime<Utc>,
) -> Result<RelayClaims, VerifyJwtError> {
    // Split into three segments.
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(VerifyJwtError::MalformedToken);
    }
    let (header_b64, payload_b64, sig_b64) = (parts[0], parts[1], parts[2]);

    // Header.
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| VerifyJwtError::MalformedHeader)?;
    let header: JwtHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| VerifyJwtError::MalformedHeader)?;
    if header.alg != ALG_EDDSA {
        return Err(VerifyJwtError::UnsupportedAlg);
    }
    let key = keys
        .iter()
        .find(|k| k.kid == header.kid)
        .ok_or(VerifyJwtError::UnknownKid)?;

    // Signature.
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| VerifyJwtError::MalformedSignature)?;
    if sig_bytes.len() != 64 {
        return Err(VerifyJwtError::MalformedSignature);
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let parsed_sig = Signature::from_bytes(&sig_arr);

    let signing_input = format!("{header_b64}.{payload_b64}");
    key.inner
        .verify(signing_input.as_bytes(), &parsed_sig)
        .map_err(|_| VerifyJwtError::InvalidSignature)?;

    // Payload (parsed only after signature verification — defends
    // against parsing tampered claims).
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| VerifyJwtError::MalformedPayload)?;
    let claims: RelayClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| VerifyJwtError::MalformedPayload)?;

    // Time checks. Targets accept up to 60s of caller-side clock skew
    // either direction so a slightly-fast-or-slow target clock
    // doesn't reject legitimate forwards.
    const SKEW_TOLERANCE_SECONDS: i64 = 60;
    let now_ts = now.timestamp();
    if claims.exp < now_ts - SKEW_TOLERANCE_SECONDS {
        return Err(VerifyJwtError::Expired);
    }
    if claims.iat > now_ts + SKEW_TOLERANCE_SECONDS {
        return Err(VerifyJwtError::NotYetValid);
    }

    Ok(claims)
}

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
    kid: String,
    #[serde(default)]
    #[allow(dead_code)]
    typ: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey as DalekSigningKey;
    use rand::rngs::OsRng;

    fn make_key(kid: &str) -> SigningKey {
        SigningKey {
            kid: kid.to_string(),
            inner: DalekSigningKey::generate(&mut OsRng),
        }
    }

    fn sample_authz() -> Authorized {
        Authorized {
            caller_user_id: Uuid::now_v7(),
            caller_account_id: Uuid::now_v7(),
            caller_agent_id: Uuid::now_v7(),
            target_account_id: Uuid::now_v7(),
            target_agent_id: Uuid::now_v7(),
            capability_id: Uuid::now_v7(),
            grant_id: Uuid::now_v7(),
            target_is_push: true,
        }
    }

    #[test]
    fn mint_then_decode_roundtrips() {
        let key = make_key("relay-2026-04");
        let authz = sample_authz();
        let now = Utc::now();
        let token = mint_for_proxied_call(&authz, &key, now, DEFAULT_TTL_SECONDS).unwrap();
        let claims = decode_relay_jwt(&token, &[key.verifying_key()], now).unwrap();
        assert_eq!(claims.iss, DEFAULT_ISSUER);
        assert_eq!(claims.aud, authz.target_agent_id.to_string());
        assert_eq!(claims.sub, authz.caller_agent_id.to_string());
        assert_eq!(claims.exp, claims.iat + DEFAULT_TTL_SECONDS);
        assert_eq!(claims.capability_id, authz.capability_id.to_string());
        assert_eq!(claims.grant_id, authz.grant_id.to_string());
    }

    #[test]
    fn token_has_three_dot_separated_segments() {
        let key = make_key("k");
        let token = mint_for_proxied_call(&sample_authz(), &key, Utc::now(), 60).unwrap();
        assert_eq!(token.matches('.').count(), 2, "JWT compact has 3 segments");
    }

    #[test]
    fn header_has_alg_eddsa_kid_and_typ_jwt() {
        let key = make_key("relay-2026-04");
        let token = mint_for_proxied_call(&sample_authz(), &key, Utc::now(), 60).unwrap();
        let header_b64 = token.split('.').next().unwrap();
        let header_bytes = URL_SAFE_NO_PAD.decode(header_b64).unwrap();
        let header: serde_json::Value = serde_json::from_slice(&header_bytes).unwrap();
        assert_eq!(header["alg"], "EdDSA");
        assert_eq!(header["kid"], "relay-2026-04");
        assert_eq!(header["typ"], "JWT");
    }

    #[test]
    fn unknown_kid_in_jwks_fails() {
        let key = make_key("relay-2026-04");
        let token = mint_for_proxied_call(&sample_authz(), &key, Utc::now(), 60).unwrap();
        let other = make_key("relay-2099-12");
        assert_eq!(
            decode_relay_jwt(&token, &[other.verifying_key()], Utc::now()),
            Err(VerifyJwtError::UnknownKid)
        );
    }

    #[test]
    fn tampered_claims_fail_signature_verification() {
        let key = make_key("k");
        let token = mint_for_proxied_call(&sample_authz(), &key, Utc::now(), 60).unwrap();

        // Substitute the payload (middle segment) with a forged one
        // claiming a different sub, leaving the signature intact.
        let parts: Vec<&str> = token.split('.').collect();
        let forged_claims = RelayClaims {
            iss: DEFAULT_ISSUER.into(),
            aud: "evil".into(),
            sub: "evil-caller".into(),
            exp: Utc::now().timestamp() + 60,
            iat: Utc::now().timestamp(),
            jti: Uuid::now_v7().to_string(),
            caller_account_id: "evil".into(),
            target_account_id: "evil".into(),
            capability_id: "evil".into(),
            grant_id: "evil".into(),
        };
        let forged_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged_claims).unwrap());
        let forged_token = format!("{}.{}.{}", parts[0], forged_b64, parts[2]);

        assert_eq!(
            decode_relay_jwt(&forged_token, &[key.verifying_key()], Utc::now()),
            Err(VerifyJwtError::InvalidSignature)
        );
    }

    #[test]
    fn tampered_signature_fails_verification() {
        let key = make_key("k");
        let token = mint_for_proxied_call(&sample_authz(), &key, Utc::now(), 60).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        // Flip a few bytes of the signature.
        let mut sig_bytes = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        sig_bytes[0] ^= 0xFF;
        let bad_sig = URL_SAFE_NO_PAD.encode(sig_bytes);
        let bad_token = format!("{}.{}.{}", parts[0], parts[1], bad_sig);
        assert_eq!(
            decode_relay_jwt(&bad_token, &[key.verifying_key()], Utc::now()),
            Err(VerifyJwtError::InvalidSignature)
        );
    }

    #[test]
    fn expired_token_rejected() {
        let key = make_key("k");
        let issued_at = Utc::now() - Duration::seconds(3600);
        let token = mint_for_proxied_call(&sample_authz(), &key, issued_at, 60).unwrap();
        let result = decode_relay_jwt(&token, &[key.verifying_key()], Utc::now());
        assert_eq!(result, Err(VerifyJwtError::Expired));
    }

    #[test]
    fn future_iat_within_skew_window_accepted() {
        let key = make_key("k");
        let issued_at = Utc::now() + Duration::seconds(30); // 30s ahead, < 60s tolerance
        let token = mint_for_proxied_call(&sample_authz(), &key, issued_at, 60).unwrap();
        assert!(decode_relay_jwt(&token, &[key.verifying_key()], Utc::now()).is_ok());
    }

    #[test]
    fn future_iat_beyond_skew_window_rejected() {
        let key = make_key("k");
        let issued_at = Utc::now() + Duration::seconds(120); // 2 min ahead
        let token = mint_for_proxied_call(&sample_authz(), &key, issued_at, 60).unwrap();
        assert_eq!(
            decode_relay_jwt(&token, &[key.verifying_key()], Utc::now()),
            Err(VerifyJwtError::NotYetValid)
        );
    }

    #[test]
    fn malformed_token_rejected() {
        let key = make_key("k");
        for bad in [
            "",
            "noseparators",
            "two.parts",
            "four.three.two.one",
            "&&&.bbb.ccc", // bad base64
        ] {
            let r = decode_relay_jwt(bad, &[key.verifying_key()], Utc::now());
            assert!(r.is_err(), "expected error for: {bad:?}");
        }
    }

    #[test]
    fn each_mint_has_unique_jti() {
        let key = make_key("k");
        let now = Utc::now();
        let t1 = mint_for_proxied_call(&sample_authz(), &key, now, 60).unwrap();
        let t2 = mint_for_proxied_call(&sample_authz(), &key, now, 60).unwrap();
        let c1 = decode_relay_jwt(&t1, &[key.verifying_key()], now).unwrap();
        let c2 = decode_relay_jwt(&t2, &[key.verifying_key()], now).unwrap();
        assert_ne!(c1.jti, c2.jti, "jti must be unique per mint for replay protection");
    }
}
