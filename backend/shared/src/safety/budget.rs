//! Spec §4.1: what Jev sees, built to fit. An oversized request would fail
//! at TypeSafe and fail open, so padding could switch screening off. Every
//! chunk's *serialized* state stays within [`STATE_BUDGET_BYTES`], sized
//! for one token per byte, however the parties pad or escape their text.

use std::collections::VecDeque;

use serde_json::{json, Map, Value};

use super::walk::At;
use super::{
    Action, Channel, Finding, FindingKind, Scan, CHUNK_DECODED_BYTES, CHUNK_TEXT_BYTES, MAX_CHUNKS,
    MAX_JUDGED_CANDIDATES, STATE_BUDGET_BYTES,
};

/// What the message is judged against. The two parties write most of it,
/// so each field is capped inside the state (§4.1).
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub capability_name: Option<String>,
    pub capability_description: Option<String>,
    pub grant_purpose: Option<String>,
    pub consent_purpose: Option<String>,
    pub proposer_message: Option<String>,
    pub response_message: Option<String>,
}

/// Caps on a field's serialized size (quotes and escaping included).
const NAME_CAP: usize = 256;
const DESCRIPTION_CAP: usize = 2048;
const MESSAGE_CAP: usize = 1024;
const PURPOSE_CAP: usize = 2048;
/// Purposes are also cut to their API limit in characters.
const PURPOSE_CHARS: usize = 500;
const PATH_CAP: usize = 120;
const CANDIDATE_TEXT_CAP: usize = 128;
const CANDIDATE_CONTEXT_CAP: usize = 200;
/// Room an entry needs beyond its text; a chunk closes below this.
const ENTRY_OVERHEAD: usize = PATH_CAP + 64;

const CUT_MARK: &str = "…[cut]";

impl Context {
    /// The context as state fields, each cut to its cap.
    fn state(&self) -> Map<String, Value> {
        let mut m = Map::new();
        let f = |s: &Option<String>, cap| s.as_deref().map(|s| fit(s, cap));
        let purpose = |s: &Option<String>| {
            s.as_deref().map(|s| {
                fit(
                    &s.chars().take(PURPOSE_CHARS).collect::<String>(),
                    PURPOSE_CAP,
                )
            })
        };
        if self.capability_name.is_some() || self.capability_description.is_some() {
            m.insert(
                "capability".into(),
                json!({
                    "name": f(&self.capability_name, NAME_CAP),
                    "description": f(&self.capability_description, DESCRIPTION_CAP),
                }),
            );
        }
        if let Some(p) = purpose(&self.grant_purpose) {
            m.insert("grant".into(), json!({ "purpose": p }));
        }
        if let Some(p) = purpose(&self.consent_purpose) {
            m.insert("consent".into(), json!({ "purpose": p }));
        }
        if self.proposer_message.is_some() || self.response_message.is_some() {
            m.insert(
                "relationship".into(),
                json!({
                    "proposal_message": f(&self.proposer_message, MESSAGE_CAP),
                    "acceptance_message": f(&self.response_message, MESSAGE_CAP),
                }),
            );
        }
        m
    }
}

/// One Jev request's state, and its field entries (for [`halve`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub state: Value,
    pub fields: Vec<Value>,
}

/// The chunks to send, and what didn't fit.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub chunks: Vec<Chunk>,
    /// Candidate ids judged one by one (in the first chunk).
    pub judged: Vec<usize>,
    /// Candidate ids past [`MAX_JUDGED_CANDIDATES`]: redacted only if
    /// `off_platform` fires (§4.3).
    pub unjudged_candidates: Vec<usize>,
    /// `unjudged` findings for text past the budget.
    pub unjudged: Vec<Finding>,
}

/// The serialized size of a value: what Jev is sent.
fn size(v: &Value) -> usize {
    serde_json::to_string(v).map_or(usize::MAX, |s| s.len())
}

/// `s`, cut on a character boundary (with a marker) so its serialized form
/// is at most `cap` bytes.
pub fn fit(s: &str, cap: usize) -> String {
    if size(&json!(s)) <= cap {
        return s.to_owned();
    }
    let end = longest_prefix(s, |end| {
        size(&json!(format!("{}{CUT_MARK}", &s[..end]))) <= cap
    });
    format!("{}{CUT_MARK}", &s[..end])
}

/// A field not yet fully placed.
struct Pending {
    item: usize,
    path: String,
    key: bool,
    text: String,
    offset: usize,
    part: u32,
}

/// Split the scan into chunks within the budget (§4.1).
pub fn plan(channel: Channel, scan: &Scan, context: &Context) -> Plan {
    let judged: Vec<usize> = scan
        .candidates
        .iter()
        .take(MAX_JUDGED_CANDIDATES)
        .map(|c| c.id)
        .collect();
    let unjudged_candidates = scan
        .candidates
        .iter()
        .skip(MAX_JUDGED_CANDIDATES)
        .map(|c| c.id)
        .collect();
    let candidates: Vec<Value> = scan
        .candidates
        .iter()
        .take(MAX_JUDGED_CANDIDATES)
        .map(|c| {
            let mut v = json!({
                "id": c.id,
                "kind": c.kind.as_str(),
                "path": fit(&scan.items[c.item(&scan.decoded)].path, PATH_CAP),
                "text": fit(&c.text, CANDIDATE_TEXT_CAP),
                "context": fit(&c.context, CANDIDATE_CONTEXT_CAP),
                "decoded": matches!(c.source, super::contacts::Source::Decoded { .. }),
            });
            if let Some(format) = scan.formats.get(c.id).and_then(Option::as_ref) {
                v["format"] = json!(fit(format, 64));
            }
            v
        })
        .collect();

    let mut queue: VecDeque<Pending> = scan
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| !i.text.is_empty())
        .map(|(n, i)| Pending {
            item: n,
            path: fit(&i.path, PATH_CAP),
            key: i.at == At::Key,
            text: i.text.clone(),
            offset: 0,
            part: 0,
        })
        .collect();
    let mut decoded_placed = vec![false; scan.decoded.len()];
    let mut unjudged_paths: Vec<String> = Vec::new();
    let context_state = context.state();
    let mut chunks = Vec::new();

    for index in 0..MAX_CHUNKS {
        if queue.is_empty() {
            break;
        }
        let mut state = Value::Object(context_state.clone());
        if index == 0 && !candidates.is_empty() {
            state["candidates"] = Value::Array(candidates.clone());
        }
        state["chunk"] = json!({ "index": index + 1, "of": MAX_CHUNKS });
        state[channel.as_str()] = json!({ "fields": [], "decoded": [] });
        let mut used = size(&state);
        let mut fields: Vec<Value> = Vec::new();
        let mut decoded: Vec<Value> = Vec::new();
        let mut text_used = 0usize;
        let mut decoded_used = 0usize;

        while let Some(p) = queue.front_mut() {
            let room = STATE_BUDGET_BYTES.saturating_sub(used);
            let text_room = CHUNK_TEXT_BYTES.saturating_sub(text_used);
            if room < ENTRY_OVERHEAD || text_room == 0 {
                break;
            }
            let rest = &p.text[p.offset..];
            let whole = entry(p, rest, p.part > 0);
            let comma = usize::from(!fields.is_empty());
            let (taken, value) = if rest.len() <= text_room && size(&whole) + comma <= room {
                (rest.len(), whole)
            } else {
                // Split: the longest prefix that fits both limits.
                let end = longest_prefix(rest, |end| {
                    end <= text_room && size(&entry(p, &rest[..end], true)) + comma <= room
                });
                if end == 0 {
                    break;
                }
                (end, entry(p, &rest[..end], true))
            };
            used += size(&value) + comma;
            text_used += taken;
            fields.push(value);
            let first_part = p.offset == 0;
            p.offset += taken;
            p.part += 1;
            let item = p.item;
            let done = p.offset >= p.text.len();
            let path = p.path.clone();
            if first_part {
                // The item's decoded spans ride along, within their budget.
                for (d, dec) in scan
                    .decoded
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.item == item)
                {
                    let room = STATE_BUDGET_BYTES.saturating_sub(used);
                    let decoded_room = CHUNK_DECODED_BYTES.saturating_sub(decoded_used);
                    let comma = usize::from(!decoded.is_empty());
                    let make = |text: &str| json!({ "path": path, "encoding": dec.encoding.as_str(), "text": text });
                    let end = longest_prefix(&dec.text, |end| {
                        end <= decoded_room && size(&make(&dec.text[..end])) + comma <= room
                    });
                    if end > 0 {
                        let v = make(&dec.text[..end]);
                        used += size(&v) + comma;
                        decoded_used += end;
                        decoded.push(v);
                        decoded_placed[d] = end == dec.text.len();
                    }
                }
            }
            if done {
                queue.pop_front();
            } else {
                break;
            }
        }

        if fields.is_empty() {
            break;
        }
        state[channel.as_str()] = json!({ "fields": fields, "decoded": decoded });
        chunks.push(Chunk { state, fields });
    }

    let n = chunks.len();
    for c in &mut chunks {
        c.state["chunk"]["of"] = json!(n);
    }
    for p in &queue {
        unjudged_paths.push(scan.items[p.item].path.clone());
    }
    for (d, placed) in decoded_placed.iter().enumerate() {
        if !placed {
            unjudged_paths.push(scan.items[scan.decoded[d].item].path.clone());
        }
    }
    unjudged_paths.dedup();
    let mut seen = std::collections::HashSet::new();
    let unjudged = unjudged_paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .map(|p| Finding::code(FindingKind::Unjudged, Action::Flagged, Some(p)))
        .collect();

    Plan {
        chunks,
        judged,
        unjudged_candidates,
        unjudged,
    }
}

fn entry(p: &Pending, text: &str, part: bool) -> Value {
    let mut v = json!({ "path": p.path, "text": text });
    if part {
        v["part"] = json!(p.part + 1);
    }
    if p.key {
        v["key"] = json!(true);
    }
    v
}

/// The largest character-boundary `end` in `s` with `ok(end)`, assuming
/// `ok` holds for every boundary below one that holds; 0 when none does.
fn longest_prefix(s: &str, ok: impl Fn(usize) -> bool) -> usize {
    let mut bounds: Vec<usize> = s.char_indices().map(|(i, _)| i).skip(1).collect();
    bounds.push(s.len());
    let (mut lo, mut hi) = (0usize, bounds.len());
    // Invariant: bounds[..lo] all ok; bounds[hi..] all not ok.
    while lo < hi {
        let mid = (lo + hi) / 2;
        if ok(bounds[mid]) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        0
    } else {
        bounds[lo - 1]
    }
}

/// For a chunk TypeSafe refused with a 4xx: the same chunk with half its
/// fields (or half its only field's text), and the entries left over for
/// the caller to place or mark `unjudged`.
pub fn halve(chunk: &Chunk, channel: Channel) -> (Chunk, Vec<Value>) {
    let mut fields = chunk.fields.clone();
    let rest = if fields.len() > 1 {
        fields.split_off(fields.len().div_ceil(2))
    } else {
        let only = fields.pop().unwrap_or(Value::Null);
        let text = only["text"].as_str().unwrap_or_default();
        let mut mid = text.len() / 2;
        while !text.is_char_boundary(mid) {
            mid -= 1;
        }
        let mut first = only.clone();
        first["text"] = json!(&text[..mid]);
        let mut second = only.clone();
        second["text"] = json!(&text[mid..]);
        let part = only["part"].as_u64().unwrap_or(1);
        first["part"] = json!(part);
        second["part"] = json!(part + 1);
        fields.push(first);
        vec![second]
    };
    let mut state = chunk.state.clone();
    state[channel.as_str()]["fields"] = Value::Array(fields.clone());
    (Chunk { state, fields }, rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::scan;
    use serde_json::json;

    fn chunks_fit(plan: &Plan) {
        for c in &plan.chunks {
            let n = size(&c.state);
            assert!(n <= STATE_BUDGET_BYTES, "chunk is {n} bytes");
        }
    }

    fn worst_context() -> Context {
        Context {
            capability_name: Some("n".repeat(10_000)),
            capability_description: Some("\u{1}".repeat(100 * 1024)),
            grant_purpose: Some("\u{2}".repeat(500)),
            consent_purpose: Some("😀".repeat(500)),
            proposer_message: Some("\"".repeat(50_000)),
            response_message: Some("\\".repeat(50_000)),
        }
    }

    #[test]
    fn a_small_message_is_one_chunk() {
        let s = scan(
            &json!({ "message": "Book a table for four on Friday." }),
            "input",
            None,
            "",
        )
        .unwrap();
        let p = plan(Channel::Request, &s, &Context::default());
        assert_eq!(p.chunks.len(), 1);
        assert!(p.unjudged.is_empty());
        assert_eq!(p.chunks[0].state["chunk"], json!({ "index": 1, "of": 1 }));
        assert_eq!(
            p.chunks[0].state["request"]["fields"][1]["text"],
            "Book a table for four on Friday."
        );
    }

    #[test]
    fn padding_gives_four_chunks_and_unjudged_text_never_an_oversized_one() {
        for pad in ["a", "\u{1}", "😀", "👨‍👩‍👧‍👦", "\"", "ह"] {
            let big = pad.repeat(200 * 1024 / pad.len());
            let value = json!({ "pad": big, "tail": "mail john@example.com" });
            let s = scan(&value, "input", None, "").unwrap();
            let p = plan(Channel::Request, &s, &worst_context());
            chunks_fit(&p);
            assert_eq!(p.chunks.len(), MAX_CHUNKS, "{pad:?}");
            assert!(
                p.unjudged
                    .iter()
                    .any(|f| f.path.as_deref() == Some("input.pad")),
                "{pad:?}: {:?}",
                p.unjudged
            );
            // The contact past the padding is still a judged candidate.
            assert_eq!(p.judged.len(), 1);
        }
    }

    #[test]
    fn many_candidates_fit_and_the_rest_are_listed() {
        let emails: Vec<String> = (0..50)
            .map(|i| format!("person{i}.{}@example.com", "x".repeat(60)))
            .collect();
        let s = scan(&json!({ "to": emails }), "input", None, "").unwrap();
        let p = plan(Channel::Request, &s, &worst_context());
        chunks_fit(&p);
        assert_eq!(p.judged.len(), MAX_JUDGED_CANDIDATES);
        assert_eq!(p.unjudged_candidates.len(), 50 - MAX_JUDGED_CANDIDATES);
        assert_eq!(
            p.chunks[0].state["candidates"].as_array().unwrap().len(),
            MAX_JUDGED_CANDIDATES
        );
    }

    #[test]
    fn decoded_text_rides_along_within_its_budget() {
        use base64::Engine;
        let hidden = base64::engine::general_purpose::STANDARD.encode("call +91 98450 12345");
        let s = scan(
            &json!({ "note": format!("ref {hidden}") }),
            "input",
            None,
            "",
        )
        .unwrap();
        let p = plan(Channel::Request, &s, &Context::default());
        let decoded = &p.chunks[0].state["request"]["decoded"];
        assert_eq!(decoded[0]["text"], "call +91 98450 12345");
        assert_eq!(decoded[0]["path"], "input.note");
    }

    #[test]
    fn context_is_capped() {
        let state = Value::Object(worst_context().state());
        assert!(size(&state["capability"]["description"]) <= DESCRIPTION_CAP);
        assert!(size(&state["relationship"]["proposal_message"]) <= MESSAGE_CAP);
        assert!(
            state["consent"]["purpose"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= PURPOSE_CHARS + CUT_MARK.chars().count()
        );
        assert!(size(&state) < STATE_BUDGET_BYTES / 2);
    }

    #[test]
    fn fit_cuts_on_character_boundaries() {
        assert_eq!(fit("short", 100), "short");
        for s in ["é".repeat(500), "\u{1}".repeat(500), "😀".repeat(500)] {
            let f = fit(&s, 100);
            assert!(size(&json!(f)) <= 100, "{}", size(&json!(f)));
            assert!(f.ends_with(CUT_MARK));
        }
    }

    #[test]
    fn halving_splits_fields_or_text() {
        let s = scan(
            &json!({ "a": "one", "b": "two", "c": "three" }),
            "input",
            None,
            "",
        )
        .unwrap();
        let p = plan(Channel::Request, &s, &Context::default());
        let (half, rest) = halve(&p.chunks[0], Channel::Request);
        assert_eq!(half.fields.len() + rest.len(), p.chunks[0].fields.len());
        assert!(size(&half.state) < size(&p.chunks[0].state));

        let s = scan(&json!("x".repeat(1000)), "input", None, "").unwrap();
        let p = plan(Channel::Request, &s, &Context::default());
        let (half, rest) = halve(&p.chunks[0], Channel::Request);
        assert_eq!(half.fields[0]["text"].as_str().unwrap().len(), 500);
        assert_eq!(rest[0]["text"].as_str().unwrap().len(), 500);
    }
}
