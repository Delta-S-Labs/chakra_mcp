//! Spec §4.3: replace contact details in the original message with
//! `[contact removed]`. Edits the original text by byte range, never the
//! normalized copy.

use std::collections::BTreeMap;
use std::ops::Range;

use serde_json::{Map, Value};

use super::contacts::Source;
use super::walk::{unescape_token, At};
use super::Scan;

pub const MARK: &str = "[contact removed]";

/// `value` with the given candidates removed. `value` is what was walked
/// (the same root), and is left untouched.
pub fn redact(value: &Value, scan: &Scan, ids: &[usize]) -> Value {
    let mut ranges: BTreeMap<usize, Vec<Range<usize>>> = BTreeMap::new();
    for c in scan.candidates.iter().filter(|c| ids.contains(&c.id)) {
        let (item, range) = match &c.source {
            Source::Item { item, range } => (*item, range.clone()),
            Source::Decoded { decoded } => {
                let d = &scan.decoded[*decoded];
                (d.item, d.range.clone())
            }
        };
        ranges.entry(item).or_default().push(range);
    }

    let mut out = value.clone();
    let mut keys: Vec<(usize, String)> = Vec::new();
    for (&i, rs) in &ranges {
        let item = &scan.items[i];
        match item.at {
            At::Str | At::Body => {
                if let Some(slot) = out.pointer_mut(&item.pointer) {
                    *slot = Value::String(redact_text(&item.text, rs));
                }
            }
            At::Num => {
                if let Some(slot) = out.pointer_mut(&item.pointer) {
                    *slot = Value::String(MARK.into());
                }
            }
            At::Key => keys.push((item.pointer.matches('/').count(), item.pointer.clone())),
        }
    }
    // Renaming a key moves everything under it, so the deepest go first.
    keys.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, pointer) in keys {
        let (parent, token) = pointer.rsplit_once('/').unwrap_or(("", &pointer));
        let key = unescape_token(token);
        if let Some(Value::Object(map)) = out.pointer_mut(parent) {
            if let Some(v) = map.remove(&key) {
                let name = free_name(map);
                map.insert(name, v);
            }
        }
    }
    out
}

/// `text` with `ranges` replaced by [`MARK`]. A text that is only the
/// contact (spacing aside) becomes just the mark.
pub fn redact_text(text: &str, ranges: &[Range<usize>]) -> String {
    let mut merged: Vec<Range<usize>> = ranges.to_vec();
    merged.sort_by_key(|r| r.start);
    let mut joined: Vec<Range<usize>> = Vec::new();
    for r in merged {
        match joined.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => joined.push(r),
        }
    }
    let trimmed_start = text.len() - text.trim_start().len();
    let trimmed_end = text.trim_end().len();
    if joined.len() == 1 && joined[0].start <= trimmed_start && joined[0].end >= trimmed_end {
        return MARK.into();
    }
    let mut out = text.to_owned();
    for r in joined.iter().rev() {
        out.replace_range(r.clone(), MARK);
    }
    out
}

/// `[contact removed 1]`, or the next number not taken in this object.
fn free_name(map: &Map<String, Value>) -> String {
    (1..)
        .map(|n| format!("[contact removed {n}]"))
        .find(|k| !map.contains_key(k))
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::scan;
    use base64::Engine;
    use serde_json::json;

    fn all(value: &Value) -> Value {
        let s = scan(value, "input", None, "").unwrap();
        let ids: Vec<usize> = s.candidates.iter().map(|c| c.id).collect();
        redact(value, &s, &ids)
    }

    #[test]
    fn replaces_in_place_and_keeps_the_rest() {
        let v = json!({ "m": "Mail john@example.com or call +91 98450 12345, thanks" });
        assert_eq!(
            all(&v),
            json!({ "m": "Mail [contact removed] or call [contact removed], thanks" })
        );
    }

    #[test]
    fn a_string_that_is_only_the_contact_becomes_the_mark() {
        assert_eq!(
            all(&json!({ "email": "  john@example.com " })),
            json!({ "email": MARK })
        );
    }

    #[test]
    fn an_encoded_contact_takes_its_whole_span() {
        let hidden = base64::engine::general_purpose::STANDARD.encode("call +91 98450 12345");
        let v = json!({ "m": format!("ref {hidden} ok") });
        assert_eq!(all(&v), json!({ "m": "ref [contact removed] ok" }));
    }

    #[test]
    fn look_alikes_are_removed_from_the_original_text() {
        let v = json!({ "m": "write jоhn@еxample.com today" });
        assert_eq!(all(&v), json!({ "m": "write [contact removed] today" }));
    }

    #[test]
    fn numbers_and_keys() {
        let v = json!({
            "phone": 9845012345u64,
            "john@example.com": "yes",
            "jane@example.com": { "nested": "call 98450 12345" },
            "[contact removed 1]": "already here",
        });
        let out = all(&v);
        assert_eq!(out["phone"], json!(MARK));
        assert_eq!(out["[contact removed 1]"], json!("already here"));
        let renamed: Vec<&String> = out
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("[contact removed ") && *k != "[contact removed 1]")
            .collect();
        assert_eq!(renamed.len(), 2, "{out}");
        // The nested value under a renamed key was edited too.
        assert!(out
            .as_object()
            .unwrap()
            .values()
            .any(|v| v == &json!({ "nested": "call [contact removed]" })));
    }

    #[test]
    fn leaves_the_input_untouched() {
        let v = json!({ "m": "john@example.com" });
        let before = v.clone();
        let _ = all(&v);
        assert_eq!(v, before);
    }
}
