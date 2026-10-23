//! Message safety, the code pass (spec `2026-10-23-message-safety-design.md`
//! §4, steps 1–4, §4.1 and §4.3's redaction).
//!
//! Everything here is deterministic and needs no model: walk a message,
//! decode what's encoded, find contact candidates and unreadable spans,
//! detect the language, split it into Jev requests that always fit, and
//! redact. The relay and the app (S2, S3) ask Jev about the result and
//! apply the policy.

pub mod budget;
pub mod contacts;
pub mod decode;
pub mod findings;
pub mod language;
pub mod metrics;
pub mod normalize;
pub mod opaque;
pub mod redact;
pub mod walk;

use serde_json::Value;

pub use budget::{fit, halve, plan, Chunk, Context, Plan};
pub use contacts::{Candidate, ContactKind, Source};
pub use decode::{Decoded, Encoding};
pub use findings::{Action, Channel, Finding, FindingKind, Safety};
pub use language::Language;
pub use opaque::{schema_at, FieldHints, Opaque};
pub use redact::{redact, redact_text, MARK};
pub use walk::{At, Item, TooLarge};

/// The most text a message may carry while screening is on: above it the
/// message is refused (`chk.safety.too_large`), never screened in part.
pub const MAX_WALK_BYTES: usize = 1024 * 1024;
/// The largest encoded span decoded.
pub const MAX_DECODE_BYTES: usize = 64 * 1024;
/// Layers of encoding decoded (base64 inside hex is two).
pub const MAX_DECODE_DEPTH: u8 = 2;
/// Message text per Jev request.
pub const CHUNK_TEXT_BYTES: usize = 16 * 1024;
/// Decoded text per Jev request.
pub const CHUNK_DECODED_BYTES: usize = 4 * 1024;
/// Jev requests per message; text past them is `unjudged`.
pub const MAX_CHUNKS: usize = 4;
/// Contact candidates judged one by one.
pub const MAX_JUDGED_CANDIDATES: usize = 16;
/// A candidate's text, as judged.
pub const CANDIDATE_BYTES: usize = 100;
/// Text around a candidate, as judged.
pub const CONTEXT_BYTES: usize = 160;
/// A Jev request's serialized state: inside 32k tokens even at one token
/// per byte, with the longest question.
pub const STATE_BUDGET_BYTES: usize = 24 * 1024;

/// Everything the code pass found in one message.
#[derive(Debug, Clone, PartialEq)]
pub struct Scan {
    pub items: Vec<Item>,
    pub decoded: Vec<Decoded>,
    pub candidates: Vec<Candidate>,
    /// Per candidate id: the schema `format` of the field it's in.
    pub formats: Vec<Option<String>>,
    pub opaque: Vec<Opaque>,
    pub language: Language,
}

impl Scan {
    /// `opaque` findings, flagged, with paths.
    pub fn opaque_findings(&self) -> Vec<Finding> {
        self.opaque
            .iter()
            .map(|o| {
                Finding::code(
                    FindingKind::Opaque,
                    Action::Flagged,
                    Some(self.items[o.item].path.clone()),
                )
            })
            .collect()
    }
}

/// Scan a JSON message. `root` prefixes display paths (`"input"`, or `""`
/// for a reply). `schema` is the capability's schema for the part of the
/// message at pointer `schema_root` (`""` for an input, `"/output"` for a
/// reply's output).
pub fn scan(
    value: &Value,
    root: &str,
    schema: Option<&Value>,
    schema_root: &str,
) -> Result<Scan, TooLarge> {
    Ok(scan_items(walk::walk(value, root)?, schema, schema_root))
}

/// Scan a push response body (see [`walk::walk_body`]).
pub fn scan_body(body: &[u8], content_type: Option<&str>) -> Result<Scan, TooLarge> {
    Ok(scan_items(walk::walk_body(body, content_type)?, None, ""))
}

fn scan_items(items: Vec<Item>, schema: Option<&Value>, schema_root: &str) -> Scan {
    let decoded = decode::decode_all(&items);
    let candidates = contacts::find(&items, &decoded);
    let formats = candidates
        .iter()
        .map(|c| {
            let item = &items[c.item(&decoded)];
            let rel = item.pointer.strip_prefix(schema_root)?;
            schema.and_then(|s| schema_at(s, rel).format)
        })
        .collect();
    let opaque = opaque::find(&items, &decoded, schema, schema_root);
    let language = language::detect(&items);
    Scan {
        items,
        decoded,
        candidates,
        formats,
        opaque,
        language,
    }
}
