//! Step 3 (spec §4): find encoded spans and decode them, so what they hide
//! is screened too. Jev reads encodings poorly; this is code's job.

use std::ops::Range;
use std::sync::LazyLock;

use base64::Engine;
use regex::Regex;

use super::walk::Item;
use super::{MAX_DECODE_BYTES, MAX_DECODE_DEPTH};

/// How a span was encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Base64,
    Hex,
    Percent,
    Escapes,
    HtmlEntities,
}

impl Encoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Base64 => "base64",
            Encoding::Hex => "hex",
            Encoding::Percent => "percent",
            Encoding::Escapes => "escapes",
            Encoding::HtmlEntities => "html_entities",
        }
    }
}

/// A decoded span.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    /// Index of the item it was found in.
    pub item: usize,
    /// The outermost encoded span in the item's original text: what
    /// redaction replaces, whatever layer the contact was found at.
    pub range: Range<usize>,
    /// 1 for a span in the original, 2 for one inside a decoded span.
    pub layer: u8,
    pub encoding: Encoding,
    pub text: String,
}

static BASE64: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/_-]{16,}={0,2}").expect("base64 regex"));
static HEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:0x)?[0-9a-f]{16,}\b").expect("hex regex"));
static PERCENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[^\s%]*%[0-9A-Fa-f]{2}){2,}[^\s%]*").expect("percent regex"));
static ESCAPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\\u[0-9A-Fa-f]{4}|\\x[0-9A-Fa-f]{2}|[^\s\\])*(?:\\u[0-9A-Fa-f]{4}|\\x[0-9A-Fa-f]{2})(?:\\u[0-9A-Fa-f]{4}|\\x[0-9A-Fa-f]{2}|[^\s\\])*")
        .expect("escapes regex")
});
static ENTITIES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:&(?:#[0-9]{1,7}|#[xX][0-9A-Fa-f]{1,6}|[A-Za-z]{2,8});|[^\s&])*&(?:#[0-9]{1,7}|#[xX][0-9A-Fa-f]{1,6}|[A-Za-z]{2,8});(?:&(?:#[0-9]{1,7}|#[xX][0-9A-Fa-f]{1,6}|[A-Za-z]{2,8});|[^\s&])*")
        .expect("entities regex")
});

/// Every readable span in the items, decoded up to [`MAX_DECODE_DEPTH`]
/// layers.
pub fn decode_all(items: &[Item]) -> Vec<Decoded> {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        for (range, encoding, text) in decode_text(&item.text) {
            nested(&mut out, i, range.clone(), 2, &text);
            out.push(Decoded {
                item: i,
                range,
                layer: 1,
                encoding,
                text,
            });
        }
    }
    out
}

fn nested(out: &mut Vec<Decoded>, item: usize, outer: Range<usize>, layer: u8, text: &str) {
    if layer > MAX_DECODE_DEPTH {
        return;
    }
    for (_, encoding, inner) in decode_text(text) {
        nested(out, item, outer.clone(), layer + 1, &inner);
        out.push(Decoded {
            item,
            range: outer.clone(),
            layer,
            encoding,
            text: inner,
        });
    }
}

/// The readable spans in one text: `(range, encoding, decoded text)`.
pub fn decode_text(text: &str) -> Vec<(Range<usize>, Encoding, String)> {
    let mut found: Vec<(Range<usize>, Encoding, String)> = Vec::new();
    let mut take = |range: Range<usize>, encoding: Encoding, decoded: Option<String>| {
        let Some(decoded) = decoded.filter(|d| readable(d) && d.as_str() != &text[range.clone()])
        else {
            return;
        };
        if found
            .iter()
            .any(|(r, _, _)| r.start < range.end && range.start < r.end)
        {
            return;
        }
        found.push((range, encoding, decoded));
    };
    for m in HEX.find_iter(text) {
        take(m.range(), Encoding::Hex, hex(m.as_str()));
    }
    for m in BASE64.find_iter(text) {
        take(m.range(), Encoding::Base64, base64(m.as_str()));
    }
    for m in PERCENT.find_iter(text) {
        take(m.range(), Encoding::Percent, percent(m.as_str()));
    }
    for m in ESCAPES.find_iter(text) {
        take(m.range(), Encoding::Escapes, escapes(m.as_str()));
    }
    for m in ENTITIES.find_iter(text) {
        take(m.range(), Encoding::HtmlEntities, entities(m.as_str()));
    }
    found.sort_by_key(|(r, _, _)| r.start);
    found
}

/// Valid text worth judging: at least 90% letters, digits, punctuation or
/// whitespace, and not empty.
fn readable(s: &str) -> bool {
    let total = s.chars().count();
    if total == 0 {
        return false;
    }
    let good = s
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || c.is_ascii_punctuation())
        .count();
    good * 10 >= total * 9
}

fn capped(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > MAX_DECODE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn base64(s: &str) -> Option<String> {
    if s.len() > MAX_DECODE_BYTES * 4 / 3 + 4 {
        return None;
    }
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let trimmed = s.trim_end_matches('=');
    [&STANDARD_NO_PAD, &URL_SAFE_NO_PAD]
        .iter()
        .find_map(|e| e.decode(trimmed).ok())
        .or_else(|| [&STANDARD, &URL_SAFE].iter().find_map(|e| e.decode(s).ok()))
        .and_then(capped)
}

fn hex(s: &str) -> Option<String> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if !s.len().is_multiple_of(2) || s.len() > MAX_DECODE_BYTES * 2 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    capped(bytes?)
}

fn percent(s: &str) -> Option<String> {
    if s.len() > MAX_DECODE_BYTES * 3 {
        return None;
    }
    capped(percent_encoding::percent_decode_str(s).collect())
}

fn escapes(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let (len, code) = if let Some(h) = tail.strip_prefix("\\u") {
            (6, u32::from_str_radix(h.get(..4)?, 16).ok()?)
        } else if let Some(h) = tail.strip_prefix("\\x") {
            (4, u32::from_str_radix(h.get(..2)?, 16).ok()?)
        } else {
            return None;
        };
        out.push(char::from_u32(code)?);
        rest = &tail[len..];
    }
    out.push_str(rest);
    (out.len() <= MAX_DECODE_BYTES).then_some(out)
}

fn entities(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let end = tail.find(';')?;
        let name = &tail[..end];
        let c = if let Some(n) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            char::from_u32(u32::from_str_radix(n, 16).ok()?)?
        } else if let Some(n) = name.strip_prefix('#') {
            char::from_u32(n.parse().ok()?)?
        } else {
            named_entity(name)?
        };
        out.push(c);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    (out.len() <= MAX_DECODE_BYTES).then_some(out)
}

/// The named entities people use to dress up an address, plus the usual.
fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "commat" => '@',
        "period" => '.',
        "colon" => ':',
        "sol" => '/',
        "bsol" => '\\',
        "plus" => '+',
        "minus" => '-',
        "hyphen" | "dash" => '-',
        "lowbar" | "UnderBar" => '_',
        "num" => '#',
        "excl" => '!',
        "quest" => '?',
        "comma" => ',',
        "semi" => ';',
        "equals" => '=',
        "percnt" => '%',
        "lpar" => '(',
        "rpar" => ')',
        "lsqb" | "lbrack" => '[',
        "rsqb" | "rbrack" => ']',
        "lcub" | "lbrace" => '{',
        "rcub" | "rbrace" => '}',
        "ast" => '*',
        "verbar" => '|',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::walk::{At, Item};

    fn item(text: &str) -> Item {
        Item {
            pointer: "/m".into(),
            path: "input.m".into(),
            at: At::Str,
            text: text.into(),
        }
    }

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    fn hexed(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn decodes_each_encoding() {
        let mail = "mail john@example.com";
        for (encoded, encoding) in [
            (b64(mail), Encoding::Base64),
            (hexed(mail), Encoding::Hex),
            ("mail%20john%40example.com".to_owned(), Encoding::Percent),
            (
                "mail\\x20john\\u0040example.com".to_owned(),
                Encoding::Escapes,
            ),
            (
                "mail&#32;john&commat;example&period;com".to_owned(),
                Encoding::HtmlEntities,
            ),
        ] {
            let text = format!("see: {encoded} thanks");
            let found = decode_text(&text);
            assert_eq!(found.len(), 1, "{encoding:?}: {found:?}");
            let (range, got_encoding, decoded) = &found[0];
            assert_eq!(*got_encoding, encoding);
            assert_eq!(decoded, mail, "{encoding:?}");
            assert_eq!(&text[range.clone()], encoded);
        }
    }

    #[test]
    fn decodes_two_layers_but_not_three() {
        let one = b64("call +91 98450 12345");
        let two = hexed(&one);
        let found = decode_all(&[item(&two)]);
        assert!(found
            .iter()
            .any(|d| d.layer == 2 && d.text == "call +91 98450 12345"));
        // Every layer points at the span in the original.
        assert!(found.iter().all(|d| d.range == (0..two.len())));

        let three = b64(&two);
        let found = decode_all(&[item(&three)]);
        assert!(!found.iter().any(|d| d.text.contains("98450")));
    }

    #[test]
    fn leaves_unreadable_and_plain_text_alone() {
        // Random bytes don't decode to readable text.
        let noise = base64::engine::general_purpose::STANDARD.encode([
            0u8, 159, 146, 150, 7, 255, 3, 1, 200, 201, 202, 203, 17, 18, 19, 20,
        ]);
        assert!(decode_text(&noise).is_empty());
        // Ordinary long words aren't base64 worth reporting.
        assert!(decode_text("internationalization and supercalifragilistic").is_empty());
        assert!(decode_text("no encodings here, just 50% off").is_empty());
    }

    #[test]
    fn caps_each_span() {
        let big = b64(&"a".repeat(MAX_DECODE_BYTES + 10));
        assert!(decode_text(&big).is_empty());
    }
}
