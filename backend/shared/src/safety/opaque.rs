//! Step 4 (spec §4): spans nobody can read — ciphertext, opaque binary —
//! that didn't decode to text. Flagged, never removed (the owner's call).

use std::ops::Range;
use std::sync::LazyLock;

use base64::Engine;
use regex::Regex;
use serde_json::Value;

use super::decode::Decoded;
use super::walk::{unescape_token, At, Item};

/// An unreadable span.
#[derive(Debug, Clone, PartialEq)]
pub struct Opaque {
    pub item: usize,
    pub range: Range<usize>,
}

/// What a JSON Schema says about a field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldHints {
    pub format: Option<String>,
    pub content_encoding: Option<String>,
    pub content_media_type: Option<String>,
}

impl FieldHints {
    /// The capability declared the field as binary content.
    pub fn binary(&self) -> bool {
        self.content_media_type.is_some()
            || self.content_encoding.as_deref().is_some_and(|e| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "base64" | "base64url" | "base16" | "base32"
                )
            })
    }
}

/// The schema node for `pointer` (relative to the schema's root),
/// following `properties`, `items`, `prefixItems` and
/// `additionalProperties`: the subset capability schemas use.
pub fn schema_at(schema: &Value, pointer: &str) -> FieldHints {
    let mut node = schema;
    for raw in pointer.split('/').skip(1) {
        let token = unescape_token(raw);
        let next = node
            .get("properties")
            .and_then(|p| p.get(&token))
            .or_else(|| {
                let i: usize = token.parse().ok()?;
                node.get("prefixItems")
                    .and_then(|p| p.get(i))
                    .or_else(|| node.get("items").filter(|v| v.is_object()))
            })
            .or_else(|| node.get("additionalProperties").filter(|v| v.is_object()));
        match next {
            Some(n) => node = n,
            None => return FieldHints::default(),
        }
    }
    let s = |k: &str| node.get(k).and_then(Value::as_str).map(str::to_owned);
    FieldHints {
        format: s("format"),
        content_encoding: s("contentEncoding"),
        content_media_type: s("contentMediaType"),
    }
}

static RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/_\-]{64,}={0,2}").expect("opaque regex"));
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").expect("uuid")
});
static JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}").expect("jwt regex")
});

/// Unreadable spans in the items. `schema` is the capability's schema for
/// the part of the message `schema_root` points at (`""` for a request's
/// input, `"/output"` for a reply).
pub fn find(
    items: &[Item],
    decoded: &[Decoded],
    schema: Option<&Value>,
    schema_root: &str,
) -> Vec<Opaque> {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if item.at == At::Num {
            continue;
        }
        let declared_binary = item.at == At::Str
            && schema.is_some_and(|schema| {
                item.pointer
                    .strip_prefix(schema_root)
                    .is_some_and(|rel| schema_at(schema, rel).binary())
            });
        if declared_binary {
            continue;
        }
        let jwts: Vec<Range<usize>> = JWT
            .find_iter(&item.text)
            .filter(|m| is_jwt(m.as_str()))
            .map(|m| m.range())
            .collect();
        for m in RUN.find_iter(&item.text) {
            let r = m.range();
            let inside = |x: &Range<usize>| x.start <= r.start && r.end <= x.end;
            let decodes = decoded
                .iter()
                .any(|d| d.item == i && d.range.start < r.end && r.start < d.range.end);
            if jwts.iter().any(inside) || decodes || !random(m.as_str(), digest_named(item)) {
                continue;
            }
            out.push(Opaque { item: i, range: r });
        }
    }
    out
}

/// Three base64url segments whose first two decode to JSON objects.
fn is_jwt(s: &str) -> bool {
    let mut parts = s.split('.');
    let json = |p: Option<&str>| {
        p.and_then(|p| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(p.trim_end_matches('='))
                .ok()
        })
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .is_some_and(|v| v.is_object())
    };
    json(parts.next()) && json(parts.next())
}

/// Words that say a hex string is a hash, not ciphertext.
const DIGEST_WORDS: &[&str] = &[
    "sha",
    "hash",
    "digest",
    "checksum",
    "commit",
    "md5",
    "etag",
    "fingerprint",
    "integrity",
    "crc",
];

/// The item's path or text names a digest (`sha256: …`, a `checksum` key).
fn digest_named(item: &Item) -> bool {
    let path = item.path.to_ascii_lowercase();
    let text = item.text.to_ascii_lowercase();
    DIGEST_WORDS
        .iter()
        .any(|w| path.contains(w) || text.contains(w))
}

/// Looks like key material or ciphertext: not a UUID or a named digest,
/// high entropy, and digits mixed in (a long identifier made of words
/// isn't). Hex of a digest's length is only a digest when something names
/// it: AES ciphertext in hex has the same lengths.
fn random(s: &str, digest_named: bool) -> bool {
    let core = s.trim_end_matches('=');
    if UUID.is_match(core) {
        return false;
    }
    let hex = core.chars().all(|c| c.is_ascii_hexdigit());
    if hex && digest_named && matches!(core.len(), 32 | 40 | 64 | 96 | 128) {
        return false;
    }
    let digits = core.chars().filter(char::is_ascii_digit).count();
    digits * 25 >= core.len() && entropy(core) >= 3.5
}

/// Shannon entropy in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
    }
    let n = s.chars().count() as f64;
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::decode::decode_all;
    use crate::safety::walk::walk;
    use serde_json::json;

    fn opaque(value: Value, schema: Option<&Value>) -> usize {
        let items = walk(&value, "input").unwrap();
        let decoded = decode_all(&items);
        find(&items, &decoded, schema, "").len()
    }

    const CIPHERTEXT: &str =
        "q9Zk3vX0Lr7Tb2Wm8Yp1Nc5Hd4Gs6Ja0Ef9Ux3Vi7Oy2Bl8Kw1Mt5Rn4Pz6Qh0Ag3Ce9Dj7Fo2Iu8Lb5Sx1";

    #[test]
    fn flags_ciphertext() {
        assert_eq!(
            opaque(json!({ "note": format!("payload: {CIPHERTEXT}") }), None),
            1
        );
        let hex_cipher =
            "9f3a0c7e1b5d2f8a4c6e0b3d7f1a5c9e2b4d6f8a0c3e5b7d9f1a3c5e7b9d0f2a4c6e8b1d3f5a";
        assert_eq!(opaque(json!({ "note": hex_cipher }), None), 1);
    }

    #[test]
    fn leaves_tokens_ids_and_digests() {
        // Built at run time: a token-shaped literal trips the secret scan.
        let part = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let jwt = format!(
            "{}.{}.{}",
            part(br#"{"alg":"EdDSA","typ":"JWT"}"#),
            part(br#"{"sub":"user-1","exp":1900000000}"#),
            part(&[7u8; 64]),
        );
        assert_eq!(opaque(json!({ "token": jwt }), None), 0);
        let sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        assert_eq!(opaque(json!({ "digest": sha256 }), None), 0);
        assert_eq!(
            opaque(json!({ "note": format!("sha256: {sha256}") }), None),
            0
        );
        // The same 64 hex characters with nothing naming a hash could be
        // two AES blocks: flagged.
        assert_eq!(opaque(json!({ "blob": sha256 }), None), 1);
        assert_eq!(
            opaque(
                json!({ "id": "9b2e6c1e-3f4a-4c2b-9d1e-1234567890ab" }),
                None
            ),
            0
        );
        // A long identifier made of words.
        assert_eq!(
            opaque(
                json!({ "k": "thisIsAVeryLongIdentifierNameThatSomeoneWroteInCamelCaseForFun" }),
                None
            ),
            0
        );
    }

    #[test]
    fn leaves_fields_declared_binary() {
        let schema = json!({
            "type": "object",
            "properties": {
                "image": { "type": "string", "contentEncoding": "base64", "contentMediaType": "image/png" },
                "files": { "type": "array", "items": { "type": "string", "contentEncoding": "base64" } }
            }
        });
        assert_eq!(
            opaque(
                json!({ "image": CIPHERTEXT, "files": [CIPHERTEXT] }),
                Some(&schema)
            ),
            0
        );
        // The same blob elsewhere is still flagged.
        assert_eq!(opaque(json!({ "note": CIPHERTEXT }), Some(&schema)), 1);
    }

    #[test]
    fn schema_lookup_follows_the_common_keywords() {
        let schema = json!({
            "properties": {
                "a": { "prefixItems": [{ "format": "email" }], "items": { "format": "uri" } },
                "m": { "additionalProperties": { "format": "date" } }
            }
        });
        assert_eq!(schema_at(&schema, "/a/0").format.as_deref(), Some("email"));
        assert_eq!(schema_at(&schema, "/a/5").format.as_deref(), Some("uri"));
        assert_eq!(
            schema_at(&schema, "/m/anything").format.as_deref(),
            Some("date")
        );
        assert_eq!(schema_at(&schema, "/missing"), FieldHints::default());
    }
}
