//! Step 1 (spec §4): the texts in a message, each with where it sits.

use serde_json::Value;

use super::MAX_WALK_BYTES;

/// What part of the JSON an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// An object member's key. `pointer` is the member's pointer.
    Key,
    /// A string value.
    Str,
    /// A phone-shaped number (7–15 digits), as text.
    Num,
    /// A response body that isn't JSON, as one text.
    Body,
}

/// One text to screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// RFC 6901 pointer to the value (or, for a key, its member), for edits.
    pub pointer: String,
    /// The display path from §5: `input.attendees[0]`.
    pub path: String,
    pub at: At,
    pub text: String,
}

/// The message's text is over [`MAX_WALK_BYTES`]: refused, never cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge;

/// Walk a JSON value. `root` prefixes display paths (`"input"` gives
/// `input.message`; `""` gives `output.slots[2]` for a reply).
pub fn walk(value: &Value, root: &str) -> Result<Vec<Item>, TooLarge> {
    let mut w = Walker::default();
    w.value(value, String::new(), root.to_owned())?;
    Ok(w.items)
}

/// Walk a push response body: JSON as JSON, an event stream event by event,
/// anything else as one text.
pub fn walk_body(body: &[u8], content_type: Option<&str>) -> Result<Vec<Item>, TooLarge> {
    if body.len() > MAX_WALK_BYTES {
        return Err(TooLarge);
    }
    let text = String::from_utf8_lossy(body);
    let mime = content_type
        .and_then(|c| c.split(';').next())
        .map(|c| c.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if mime == "text/event-stream" {
        let mut w = Walker::default();
        let events = text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim_start);
        for (i, data) in events.enumerate() {
            let pointer = format!("/events/{i}");
            let path = format!("events[{i}]");
            match serde_json::from_str::<Value>(data) {
                Ok(v) => w.value(&v, pointer, path)?,
                Err(_) => w.push(pointer, path, At::Body, data.to_owned())?,
            }
        }
        return Ok(w.items);
    }
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        return walk(&v, "");
    }
    let mut w = Walker::default();
    w.push(String::new(), "body".into(), At::Body, text.into_owned())?;
    Ok(w.items)
}

#[derive(Default)]
struct Walker {
    items: Vec<Item>,
    bytes: usize,
}

impl Walker {
    fn push(
        &mut self,
        pointer: String,
        path: String,
        at: At,
        text: String,
    ) -> Result<(), TooLarge> {
        self.bytes += text.len();
        if self.bytes > MAX_WALK_BYTES {
            return Err(TooLarge);
        }
        self.items.push(Item {
            pointer,
            path,
            at,
            text,
        });
        Ok(())
    }

    fn value(&mut self, value: &Value, pointer: String, path: String) -> Result<(), TooLarge> {
        match value {
            Value::String(s) => self.push(pointer, path, At::Str, s.clone()),
            Value::Number(n) => match phone_shaped(n) {
                Some(digits) => self.push(pointer, path, At::Num, digits),
                None => Ok(()),
            },
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    self.value(v, format!("{pointer}/{i}"), format!("{path}[{i}]"))?;
                }
                Ok(())
            }
            Value::Object(map) => {
                for (k, v) in map {
                    let member = format!("{pointer}/{}", escape_token(k));
                    let member_path = join(&path, k);
                    self.push(member.clone(), member_path.clone(), At::Key, k.clone())?;
                    self.value(v, member, member_path)?;
                }
                Ok(())
            }
            Value::Null | Value::Bool(_) => Ok(()),
        }
    }
}

/// An integer of 7–15 digits, as text: a phone number can be a JSON number.
fn phone_shaped(n: &serde_json::Number) -> Option<String> {
    let digits = n.as_u64()?.to_string();
    (7..=15).contains(&digits.len()).then_some(digits)
}

/// `a.b`, or `a["odd key"]` when the key isn't a plain identifier.
fn join(path: &str, key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    match (path.is_empty(), plain) {
        (true, true) => key.to_owned(),
        (false, true) => format!("{path}.{key}"),
        (_, false) => format!("{path}[{}]", Value::String(key.to_owned())),
    }
}

/// RFC 6901: `~` → `~0`, `/` → `~1`.
pub(crate) fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// The inverse of [`escape_token`].
pub(crate) fn unescape_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn walks_keys_strings_and_phone_shaped_numbers() {
        let v = json!({
            "message": "hi",
            "attendees": ["a@example.com"],
            "count": 3,
            "phone": 9845012345u64,
            "ok": true,
            "odd/key": "x",
        });
        let items = walk(&v, "input").unwrap();
        let got: Vec<(&str, &str, At, &str)> = items
            .iter()
            .map(|i| (i.pointer.as_str(), i.path.as_str(), i.at, i.text.as_str()))
            .collect();
        assert!(got.contains(&("/message", "input.message", At::Str, "hi")));
        assert!(got.contains(&("/message", "input.message", At::Key, "message")));
        assert!(got.contains(&(
            "/attendees/0",
            "input.attendees[0]",
            At::Str,
            "a@example.com"
        )));
        assert!(got.contains(&("/phone", "input.phone", At::Num, "9845012345")));
        assert!(got.contains(&("/odd~1key", "input[\"odd/key\"]", At::Str, "x")));
        // Short numbers and booleans aren't walked.
        assert!(!got.iter().any(|g| g.0 == "/count" && g.2 != At::Key));
        assert!(!got.iter().any(|g| g.0 == "/ok" && g.2 != At::Key));
    }

    #[test]
    fn refuses_over_the_cap_and_never_truncates() {
        let at_cap = json!("x".repeat(MAX_WALK_BYTES));
        assert_eq!(walk(&at_cap, "").unwrap().len(), 1);
        let over = json!("x".repeat(MAX_WALK_BYTES + 1));
        assert_eq!(walk(&over, ""), Err(TooLarge));
        // Keys count too.
        let keys = json!({ "k".repeat(MAX_WALK_BYTES): "v" });
        assert_eq!(walk(&keys, ""), Err(TooLarge));
    }

    #[test]
    fn walks_bodies_by_type() {
        let sse = b"event: message\ndata: {\"text\": \"hello\"}\n\ndata: plain words\n\n";
        let items = walk_body(sse, Some("text/event-stream; charset=utf-8")).unwrap();
        let texts: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.path.as_str(), i.text.as_str()))
            .collect();
        assert!(texts.contains(&("events[0].text", "hello")));
        assert!(texts.contains(&("events[1]", "plain words")));

        let json_body = br#"{"result": {"text": "hi"}}"#;
        let items = walk_body(json_body, Some("application/json")).unwrap();
        assert!(items
            .iter()
            .any(|i| i.path == "result.text" && i.text == "hi"));

        let plain = walk_body(b"just text", Some("text/plain")).unwrap();
        assert_eq!(plain[0].path, "body");
        assert_eq!(plain[0].at, At::Body);

        assert_eq!(
            walk_body(&vec![b'a'; MAX_WALK_BYTES + 1], None),
            Err(TooLarge)
        );
    }

    #[test]
    fn pointer_tokens_round_trip() {
        for t in ["a/b", "a~b", "~1", "plain"] {
            assert_eq!(unescape_token(&escape_token(t)), t);
        }
    }
}
