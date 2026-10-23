//! Consent tokens (spec §6): a signed statement of what an approver
//! approved, bound to the exact value delivered by its SHA-256.
//!
//! An EdDSA JWT signed with the relay's signing key (`relay_signing_keys`,
//! published at `/.well-known/jwks.json`). The same key signs the 60-second
//! push bearers (`typ: "JWT"`) and Agent Card JWS (no `typ`), so verifiers
//! must check `typ`. The token grants nothing in the relay; the owner's own
//! systems can check it offline.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer as _, Verifier as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_card::signer::{SigningKey, VerifyingKey, ALG_EDDSA};

/// The JWT `typ` header of a consent token.
pub const CONSENT_TYP: &str = "chakramcp-consent+jwt";

/// A request token lasts 24 hours from delivery.
pub const REQUEST_TOKEN_TTL_SECS: i64 = 24 * 60 * 60;
/// A reply token lasts 30 days from its first read.
pub const REPLY_TOKEN_TTL_SECS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsentClaims {
    /// The relay's public URL; its JWKS is at `{iss}/.well-known/jwks.json`.
    pub iss: String,
    /// The consent request id.
    pub jti: String,
    /// `request` (gate 1) or `reply` (gate 2).
    pub kind: String,
    /// The invocation id.
    pub inv: String,
    /// The capability id.
    pub cap: String,
    /// Target agent id.
    pub granter: String,
    /// Requesting agent id.
    pub grantee: String,
    /// The approving user's id.
    pub approver: String,
    /// The standing consent that covered the request, if one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<String>,
    /// Hex SHA-256 of the RFC 8785 (JCS) canonical JSON of the value
    /// delivered: the input (`request`) or the released output (`reply`).
    pub sha256: String,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TokenError {
    #[error("malformed token")]
    Malformed,
    #[error("not a consent token")]
    WrongType,
    #[error("unknown signing key")]
    UnknownKid,
    #[error("bad signature")]
    BadSignature,
    #[error("token expired")]
    Expired,
    #[error("the value doesn't match the token's sha256")]
    ValueMismatch,
}

pub use chakramcp_shared::consent::hash::sha256_jcs;

/// Sign `claims` as a consent token.
pub fn mint(claims: &ConsentClaims, key: &SigningKey) -> Result<String, serde_json::Error> {
    let header = serde_json::json!({ "alg": ALG_EDDSA, "kid": key.kid, "typ": CONSENT_TYP });
    let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header)?);
    let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims)?);
    let signing_input = format!("{header_b64}.{payload_b64}");
    let sig: Signature = key.inner.sign(signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(sig.to_bytes())
    ))
}

/// Verify a consent token against JWKS keys, as the SDKs' helper does: the
/// `typ`, the signature, the expiry and, given `value`, its hash.
pub fn verify(
    token: &str,
    keys: &[VerifyingKey],
    now: DateTime<Utc>,
    value: Option<&Value>,
) -> Result<ConsentClaims, TokenError> {
    #[derive(Deserialize)]
    struct Header {
        alg: String,
        kid: String,
        typ: Option<String>,
    }
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(sig_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TokenError::Malformed);
    };
    let decode = |s: &str| URL_SAFE_NO_PAD.decode(s).map_err(|_| TokenError::Malformed);
    let header: Header =
        serde_json::from_slice(&decode(header_b64)?).map_err(|_| TokenError::Malformed)?;
    if header.typ.as_deref() != Some(CONSENT_TYP) || header.alg != ALG_EDDSA {
        return Err(TokenError::WrongType);
    }
    let key = keys
        .iter()
        .find(|k| k.kid == header.kid)
        .ok_or(TokenError::UnknownKid)?;
    let sig: [u8; 64] = decode(sig_b64)?
        .try_into()
        .map_err(|_| TokenError::Malformed)?;
    key.inner
        .verify(
            format!("{header_b64}.{payload_b64}").as_bytes(),
            &Signature::from_bytes(&sig),
        )
        .map_err(|_| TokenError::BadSignature)?;
    let claims: ConsentClaims =
        serde_json::from_slice(&decode(payload_b64)?).map_err(|_| TokenError::Malformed)?;
    if claims.exp <= now.timestamp() {
        return Err(TokenError::Expired);
    }
    if let Some(value) = value {
        if sha256_jcs(value) != claims.sha256 {
            return Err(TokenError::ValueMismatch);
        }
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(kid: &str) -> SigningKey {
        SigningKey::from_bytes(kid.into(), &[7u8; 32])
    }

    fn claims(input: &Value, now: i64) -> ConsentClaims {
        ConsentClaims {
            iss: "https://relay.example".into(),
            jti: "req".into(),
            kind: "request".into(),
            inv: "inv".into(),
            cap: "cap".into(),
            granter: "a".into(),
            grantee: "b".into(),
            approver: "u".into(),
            consent: None,
            sha256: sha256_jcs(input),
            iat: now,
            exp: now + REQUEST_TOKEN_TTL_SECS,
        }
    }

    #[test]
    fn canonical_hash_ignores_key_order_and_whitespace() {
        let a: Value =
            serde_json::from_str(r#"{"b": 1, "a": [1, 2, {"d": "x", "c": null}]}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":[1,2,{"c":null,"d":"x"}],"b":1}"#).unwrap();
        assert_eq!(sha256_jcs(&a), sha256_jcs(&b));
        assert_ne!(sha256_jcs(&a), sha256_jcs(&json!({"b": 2})));
    }

    /// RFC 8785 appendix vectors: number serialization (B) and key sorting
    /// by UTF-16 code units (3.2.3), so every SDK hashes the same bytes.
    #[test]
    fn jcs_matches_rfc_8785_vectors() {
        let vectors = include_str!("../../tests/data/jcs/vectors.json");
        let cases: Vec<(String, String)> = serde_json::from_str(vectors).unwrap();
        assert!(!cases.is_empty());
        for (input, canonical) in cases {
            let value: Value = serde_json::from_str(&input).unwrap();
            let got = chakramcp_shared::consent::hash::canonical_json(&value);
            assert_eq!(got, canonical, "input {input}");
        }
    }

    #[test]
    fn minted_tokens_verify_and_bind_the_value() {
        let k = key("kid-1");
        let keys = [k.verifying_key()];
        let input = json!({"date": "2026-10-24", "slots": [1, 2]});
        let now = Utc::now();
        let token = mint(&claims(&input, now.timestamp()), &k).unwrap();

        let got = verify(&token, &keys, now, Some(&input)).unwrap();
        assert_eq!(got.kind, "request");
        assert_eq!(
            verify(&token, &keys, now, Some(&json!({"date": "2026-10-25"}))),
            Err(TokenError::ValueMismatch)
        );
        let later = now + chrono::Duration::seconds(REQUEST_TOKEN_TTL_SECS + 1);
        assert_eq!(verify(&token, &keys, later, None), Err(TokenError::Expired));
        assert_eq!(
            verify(&token, &[key("other").verifying_key()], now, None),
            Err(TokenError::UnknownKid)
        );
    }

    #[test]
    fn push_bearers_are_not_consent_tokens() {
        let k = key("kid-1");
        let relay_claims = crate::jwt_mint::RelayClaims {
            iss: "https://chakramcp.com".into(),
            aud: "a".into(),
            sub: "b".into(),
            exp: Utc::now().timestamp() + 60,
            iat: Utc::now().timestamp(),
            jti: "j".into(),
            caller_account_id: "x".into(),
            target_account_id: "y".into(),
            capability_id: "c".into(),
            grant_id: "g".into(),
        };
        let bearer = crate::jwt_mint::mint_jwt(&relay_claims, &k).unwrap();
        assert_eq!(
            verify(&bearer, &[k.verifying_key()], Utc::now(), None),
            Err(TokenError::WrongType)
        );
    }
}
