//! Ed25519 JWS signing + verification for Agent Cards.
//!
//! Every public Agent Card we publish carries one or more
//! `AgentCardSignature` entries (RFC 7515 JWS shape). The signature
//! is computed over a deterministic JSON projection of the card body
//! (the body sans `signatures` field — signing one's own signature
//! is impossible).
//!
//! Wire shape per `AgentCardSignature`:
//! ```json
//! {
//!   "protected": "<base64url(JSON({\"alg\":\"EdDSA\",\"kid\":\"<kid>\"}))>",
//!   "signature": "<base64url(Ed25519(protected '.' payload))>"
//! }
//! ```
//!
//! Where `payload` is `base64url(json(card_without_signatures))`. This
//! follows JWS Compact form's "signing input" pattern:
//! `BASE64URL(protected) || '.' || BASE64URL(payload)` (RFC 7515 §5.1).
//!
//! Determinism: `serde_json::Map` is BTreeMap-backed by default
//! (sorted keys); `BTreeMap<String, SecurityScheme>` in `AgentCard`
//! is also sorted; structs serialize in field declaration order.
//! Combined, our serialized payload is byte-stable across runs and
//! processes — verifiers re-serialize the parsed card and get the
//! same bytes. Future hardening (RFC 8785 JCS) is not needed for v1
//! because we control both ends today.
//!
//! Threat model:
//! - The private key never leaves this process. Public keys are
//!   published in JWKS at `/.well-known/jwks.json`.
//! - The signature covers `supported_interfaces` (including the URL
//!   we publish), so a card replayed under a different URL fails
//!   verification.
//! - A tampered field anywhere in the payload changes the canonical
//!   bytes; the signature won't verify.
//! - Forward-compat (`extra` maps) participates in the signed
//!   projection: if a future reviewer adds an unknown field to a
//!   parsed card and re-signs, the new signature covers it.
//!
//! What's NOT here (deferred):
//! - Multi-process leader election for key rotation (D2e).
//! - Encryption-at-rest for `private_key_bytes` in the DB (TODO in
//!   discovery spec; v1 relies on DB password + Postgres TLS).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{
    Signature, Signer as _, SigningKey as DalekSigningKey, Verifier as _,
    VerifyingKey as DalekVerifyingKey, SECRET_KEY_LENGTH,
};
use serde::Serialize;
use serde_json::Value;

use super::types::{AgentCard, AgentCardSignature};

/// Algorithm identifier emitted in JWS protected headers and JWKS.
pub const ALG_EDDSA: &str = "EdDSA";

/// JOSE `kty` for Ed25519 (RFC 8037).
pub const KTY_OKP: &str = "OKP";

/// JOSE `crv` for Ed25519 (RFC 8037).
pub const CRV_ED25519: &str = "Ed25519";

/// Signing key + its public kid. Constructed from the DB row by
/// `keys::SigningKeySet::current()`. Holds the private material in
/// memory only; ZeroizeOnDrop is provided by `ed25519-dalek`.
pub struct SigningKey {
    pub kid: String,
    pub inner: DalekSigningKey,
}

impl SigningKey {
    /// Construct from raw 32-byte secret material, e.g. as fetched
    /// from the DB.
    pub fn from_bytes(kid: String, bytes: &[u8; SECRET_KEY_LENGTH]) -> Self {
        Self {
            kid,
            inner: DalekSigningKey::from_bytes(bytes),
        }
    }

    /// Public half, for distribution in JWKS.
    pub fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey {
            kid: self.kid.clone(),
            inner: self.inner.verifying_key(),
        }
    }
}

/// Public verification key + its kid. Distributed via JWKS.
#[derive(Debug, Clone)]
pub struct VerifyingKey {
    pub kid: String,
    pub inner: DalekVerifyingKey,
}

#[derive(Debug, thiserror::Error)]
pub enum SignError {
    #[error("failed to serialize card payload: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum VerifyError {
    #[error("card has no signatures")]
    NoSignatures,
    #[error("no signature on card matched a published key (kid not in JWKS)")]
    UnknownKid,
    #[error("signature could not be parsed (not 64 base64url bytes)")]
    MalformedSignature,
    #[error("protected header could not be parsed (not base64url JSON)")]
    MalformedProtectedHeader,
    #[error("protected header is missing required fields (alg, kid)")]
    IncompleteProtectedHeader,
    #[error("alg in protected header is not EdDSA")]
    UnsupportedAlg,
    #[error("payload could not be canonicalized: {0}")]
    Canonicalize(String),
    #[error("signature did not verify against the matching public key")]
    InvalidSignature,
}

/// Sign `card` with `key`, append the resulting `AgentCardSignature`
/// to `card.signatures`, and return the signed card.
///
/// Existing signatures (e.g. from upstream republish) are preserved.
/// Multiple signatures per card are explicitly allowed by the A2A
/// spec.
pub fn sign_card(card: &mut AgentCard, key: &SigningKey) -> Result<(), SignError> {
    let payload_b64 = canonical_payload_b64(card)?;
    let protected_b64 = protected_header_b64(&key.kid);
    let signing_input = format!("{protected_b64}.{payload_b64}");
    let signature: Signature = key.inner.sign(signing_input.as_bytes());
    let signature_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

    card.signatures.push(AgentCardSignature {
        protected: protected_b64,
        signature: signature_b64,
        header: None,
        extra: Default::default(),
    });
    Ok(())
}

/// Verify that `card` carries at least one signature signed by one of
/// the `keys` listed in our JWKS. Returns Ok on first match; returns
/// Err if NO signature on the card is verifiable.
///
/// Cards may carry signatures from multiple parties (relay + upstream
/// canonical). This function only attests "the relay (or one of the
/// listed keys) signed this card." Other parties' signatures are
/// ignored — verifying them is the caller's responsibility.
pub fn verify_card(card: &AgentCard, keys: &[VerifyingKey]) -> Result<(), VerifyError> {
    if card.signatures.is_empty() {
        return Err(VerifyError::NoSignatures);
    }

    // Build the canonical payload ONCE — it's the same regardless of
    // which signature we're checking.
    let payload_b64 =
        canonical_payload_b64(card).map_err(|e| VerifyError::Canonicalize(e.to_string()))?;

    for sig in &card.signatures {
        // A card may carry signatures from multiple parties (upstream
        // + ChakraMCP). Signatures whose `protected` doesn't decode
        // as base64url-JSON-with-alg-EdDSA aren't from us — skip them
        // rather than erroring; the next one might be ours. Same for
        // a kid that doesn't appear in the supplied key set.
        let Ok(header_bytes) = URL_SAFE_NO_PAD.decode(&sig.protected) else {
            continue;
        };
        let Ok(header) = serde_json::from_slice::<ProtectedHeader>(&header_bytes) else {
            continue;
        };
        if header.alg != ALG_EDDSA {
            continue;
        }
        let Some(kid) = header.kid.as_deref() else {
            continue;
        };
        let Some(key) = keys.iter().find(|k| k.kid == kid) else {
            continue;
        };

        // Found a candidate — from this point on, errors are real.
        let sig_bytes = URL_SAFE_NO_PAD
            .decode(&sig.signature)
            .map_err(|_| VerifyError::MalformedSignature)?;
        if sig_bytes.len() != 64 {
            return Err(VerifyError::MalformedSignature);
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let parsed_sig = Signature::from_bytes(&sig_arr);

        let signing_input = format!("{}.{}", sig.protected, payload_b64);
        return if key.inner.verify(signing_input.as_bytes(), &parsed_sig).is_ok() {
            Ok(())
        } else {
            Err(VerifyError::InvalidSignature)
        };
    }

    // No signature on the card was attributable to a key we know
    // about — could mean the card has only upstream signatures, or
    // the kid we expected isn't in our JWKS yet.
    Err(VerifyError::UnknownKid)
}

#[derive(Serialize, serde::Deserialize)]
struct ProtectedHeader {
    alg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
}

fn protected_header_b64(kid: &str) -> String {
    // Hand-build the JSON to ensure we don't accidentally introduce
    // whitespace or key-ordering instability. The protected header is
    // small + fixed-shape; this is more robust than serde_json here.
    let json = format!(r#"{{"alg":"{ALG_EDDSA}","kid":"{kid}"}}"#);
    URL_SAFE_NO_PAD.encode(json.as_bytes())
}

/// Canonical-JSON serialization of the card body MINUS its signatures
/// field. base64url-no-pad encoded as the JWS payload.
fn canonical_payload_b64(card: &AgentCard) -> Result<String, serde_json::Error> {
    // Project to a Value, drop signatures, serialize. Our types use
    // BTreeMap / serde_json::Map (ordered) for every map, and structs
    // serialize fields in declaration order — combined, the serialized
    // bytes are deterministic across runs/processes for a given
    // logical card.
    let mut value = serde_json::to_value(card)?;
    if let Value::Object(ref mut map) = value {
        map.remove("signatures");
    }
    let bytes = serde_json::to_vec(&value)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::super::synthesizer::*;
    use super::*;
    use rand::rngs::OsRng;

    fn make_key(kid: &str) -> SigningKey {
        let inner = DalekSigningKey::generate(&mut OsRng);
        SigningKey {
            kid: kid.to_string(),
            inner,
        }
    }

    fn sample_card() -> AgentCard {
        synthesize_pull_card(
            &AgentRowForSynthesis {
                account_slug: "acme-corp".into(),
                agent_slug: "alice".into(),
                display_name: "Alice".into(),
                description: "Demo agent.".into(),
                agent_version: "0.1.0".into(),
            },
            &[CapabilityRowForSynthesis {
                id: "cap-1".into(),
                name: "do-thing".into(),
                description: "Does a thing.".into(),
            }],
            "https://chakramcp.com",
        )
        .unwrap()
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let mut card = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut card, &key).unwrap();
        assert_eq!(card.signatures.len(), 1);

        // Verify with the published public key.
        let pub_keys = vec![key.verifying_key()];
        assert!(verify_card(&card, &pub_keys).is_ok());
    }

    #[test]
    fn signature_has_jws_shape() {
        let mut card = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut card, &key).unwrap();
        let sig = &card.signatures[0];

        // protected header must decode as JSON with alg+kid.
        let header_bytes = URL_SAFE_NO_PAD.decode(&sig.protected).unwrap();
        let header: serde_json::Value = serde_json::from_slice(&header_bytes).unwrap();
        assert_eq!(header["alg"], "EdDSA");
        assert_eq!(header["kid"], "relay-2026-04");

        // Signature bytes are 64 (Ed25519 signature length).
        let sig_bytes = URL_SAFE_NO_PAD.decode(&sig.signature).unwrap();
        assert_eq!(sig_bytes.len(), 64);
    }

    #[test]
    fn unknown_kid_in_jwks_does_not_verify() {
        let mut card = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut card, &key).unwrap();

        let other_key = make_key("relay-2099-12");
        let pub_keys = vec![other_key.verifying_key()];
        assert_eq!(verify_card(&card, &pub_keys), Err(VerifyError::UnknownKid));
    }

    #[test]
    fn tampered_name_fails_verification() {
        let mut card = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut card, &key).unwrap();
        card.name = "EvilAlice".to_string();

        let pub_keys = vec![key.verifying_key()];
        assert_eq!(
            verify_card(&card, &pub_keys),
            Err(VerifyError::InvalidSignature)
        );
    }

    #[test]
    fn tampered_url_fails_verification() {
        let mut card = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut card, &key).unwrap();
        card.supported_interfaces[0].url =
            "https://evil.example.com/agents/alice/a2a/jsonrpc".to_string();

        let pub_keys = vec![key.verifying_key()];
        assert_eq!(
            verify_card(&card, &pub_keys),
            Err(VerifyError::InvalidSignature)
        );
    }

    #[test]
    fn tampered_skill_fails_verification() {
        let mut card = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut card, &key).unwrap();
        card.skills[0].description = "Now does a different thing.".to_string();

        let pub_keys = vec![key.verifying_key()];
        assert_eq!(
            verify_card(&card, &pub_keys),
            Err(VerifyError::InvalidSignature)
        );
    }

    #[test]
    fn no_signatures_is_an_error() {
        let card = sample_card();
        assert_eq!(
            verify_card(&card, &[make_key("x").verifying_key()]),
            Err(VerifyError::NoSignatures)
        );
    }

    #[test]
    fn verify_picks_matching_kid_among_multiple_signatures() {
        // Card carries two signatures: one from us, one from a
        // hypothetical upstream signer with a kid we don't recognize.
        // verify_card should match OUR kid and return Ok.
        let mut card = sample_card();
        let our_key = make_key("relay-2026-04");
        let upstream_key = make_key("upstream-something");

        sign_card(&mut card, &upstream_key).unwrap();
        sign_card(&mut card, &our_key).unwrap();
        assert_eq!(card.signatures.len(), 2);

        // We only publish our_key's public half in JWKS. Verification
        // should still succeed because verify_card iterates and finds
        // the matching kid.
        let pub_keys = vec![our_key.verifying_key()];
        assert!(verify_card(&card, &pub_keys).is_ok());
    }

    #[test]
    fn signing_is_deterministic_payload_unstable_signature_is_ok() {
        // Ed25519 signatures are deterministic per RFC 8032 — same
        // key + same input -> same signature bytes. Our payload is
        // canonical, so two signs of the same logical card produce
        // byte-identical signatures.
        let mut a = sample_card();
        let mut b = sample_card();
        let key = make_key("relay-2026-04");
        sign_card(&mut a, &key).unwrap();
        sign_card(&mut b, &key).unwrap();
        assert_eq!(a.signatures[0].signature, b.signatures[0].signature);
        assert_eq!(a.signatures[0].protected, b.signatures[0].protected);
    }

    #[test]
    fn signing_preserves_existing_signatures() {
        let mut card = sample_card();
        let upstream_key = make_key("upstream-old");
        sign_card(&mut card, &upstream_key).unwrap();
        assert_eq!(card.signatures.len(), 1);

        let our_key = make_key("relay-2026-04");
        sign_card(&mut card, &our_key).unwrap();
        assert_eq!(card.signatures.len(), 2);

        // Both verifiable with the right key set.
        assert!(verify_card(
            &card,
            &[upstream_key.verifying_key(), our_key.verifying_key()]
        )
        .is_ok());
    }
}
