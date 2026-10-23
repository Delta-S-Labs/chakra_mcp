//! The hash a consent token binds: SHA-256 over the RFC 8785 (JCS)
//! canonical JSON of the value delivered, so every implementation (the
//! relay, each SDK) hashes the same bytes. Test vectors:
//! `relay/tests/data/jcs/vectors.json`.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The canonical JSON (RFC 8785) of `value`.
pub fn canonical_json(value: &Value) -> String {
    // Serializing a `Value` can't fail: it has no non-string keys or
    // non-finite numbers.
    serde_json_canonicalizer::to_string(value).unwrap_or_default()
}

/// Hex SHA-256 of the canonical JSON of `value`.
pub fn sha256_jcs(value: &Value) -> String {
    hex::encode(Sha256::digest(canonical_json(value).as_bytes()))
}
