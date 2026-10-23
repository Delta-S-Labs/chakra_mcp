//! Step 2 (spec §4): a folded copy of a text for detection, with a map back
//! to the original's bytes so redaction can edit the original.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;

/// A text folded for detection.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    pub text: String,
    /// `map[i]` is the original byte offset of normalized byte `i`; one
    /// extra entry holds the original's length.
    map: Vec<usize>,
}

impl Normalized {
    /// The original byte range a normalized range came from.
    pub fn original(&self, range: Range<usize>) -> Range<usize> {
        self.map[range.start]..self.map[range.end]
    }
}

/// Fold `s`: NFKC, zero-width and bidi controls dropped, look-alike letters
/// and Unicode decimal digits mapped to ASCII, lowercased. One character
/// at a time, which is enough for detection; redaction never uses it.
pub fn normalize(s: &str) -> Normalized {
    let mut text = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len() + 1);
    for (at, c) in s.char_indices() {
        if invisible(c) {
            continue;
        }
        for d in c.nfkc() {
            for folded in fold(d).to_lowercase() {
                let before = text.len();
                text.push(folded);
                map.extend(std::iter::repeat_n(at, text.len() - before));
            }
        }
    }
    map.push(s.len());
    Normalized { text, map }
}

/// Characters that render as nothing and can hide inside an address.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// Look-alike letters (Cyrillic, Greek) and decimal digits from other
/// scripts, mapped to ASCII. Mathematical and fullwidth forms are already
/// handled by NFKC.
fn fold(c: char) -> char {
    if let Some(d) = decimal_digit(c) {
        return d;
    }
    match c {
        'а' | 'А' | 'α' | 'Α' => 'a',
        'в' | 'В' | 'β' | 'Β' => 'b',
        'с' | 'С' | 'ϲ' => 'c',
        'ԁ' => 'd',
        'е' | 'Е' | 'ε' | 'Ε' => 'e',
        'һ' | 'Н' | 'Η' => 'h',
        'і' | 'І' | 'ι' | 'Ι' | 'ӏ' => 'i',
        'ј' | 'Ј' => 'j',
        'к' | 'К' | 'κ' | 'Κ' => 'k',
        'м' | 'М' | 'Μ' => 'm',
        'п' | 'η' | 'Ν' => 'n',
        'о' | 'О' | 'ο' | 'Ο' | 'σ' => 'o',
        'р' | 'Р' | 'ρ' | 'Ρ' => 'p',
        'ԛ' => 'q',
        'ѕ' | 'Ѕ' => 's',
        'т' | 'Т' | 'τ' | 'Τ' => 't',
        'υ' | 'ս' => 'u',
        'ν' | 'ѵ' => 'v',
        'ԝ' | 'ѡ' | 'ω' => 'w',
        'х' | 'Х' | 'χ' | 'Χ' => 'x',
        'у' | 'У' | 'γ' | 'Υ' => 'y',
        'Ζ' => 'z',
        _ => c,
    }
}

/// A decimal digit from any script (Devanagari `५`, Bengali `৫`,
/// Arabic-Indic `٥`…) as its ASCII digit.
fn decimal_digit(c: char) -> Option<char> {
    if c.is_ascii_digit() || !c.is_numeric() {
        return None;
    }
    // Unicode lays decimal digits out as runs of ten starting at zero.
    const ZEROS: &[u32] = &[
        0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
        0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0xFF10,
    ];
    let cp = c as u32;
    ZEROS
        .iter()
        .find(|&&zero| (zero..zero + 10).contains(&cp))
        .and_then(|zero| char::from_digit(cp - zero, 10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_what_hides_an_address() {
        assert_eq!(
            normalize("jo\u{200B}hn@exa\u{FEFF}mple.com").text,
            "john@example.com"
        );
        // Cyrillic о, е and а.
        assert_eq!(normalize("jоhn@еxаmple.com").text, "john@example.com");
        // Fullwidth letters and digits (NFKC).
        assert_eq!(normalize("ＪＯＨＮ９８").text, "john98");
        assert_eq!(normalize("९८४५०").text, "98450");
        assert_eq!(normalize("৯৮৪৫০").text, "98450");
        assert_eq!(normalize("٩٨").text, "98");
    }

    #[test]
    fn maps_spans_back_to_the_original() {
        let original = "Mail: ＪＯＨＮ\u{200B}@example.com!";
        let n = normalize(original);
        let start = n.text.find("john@").unwrap();
        let end = start + "john@example.com".len();
        let back = n.original(start..end);
        assert_eq!(&original[back], "ＪＯＨＮ\u{200B}@example.com");
        // The whole text maps to the whole original.
        assert_eq!(n.original(0..n.text.len()), 0..original.len());
    }

    #[test]
    fn leaves_ordinary_text_alone_apart_from_case() {
        assert_eq!(normalize("Hello, World 42").text, "hello, world 42");
        assert_eq!(normalize("नमस्ते").text, "नमस्ते".nfkc().collect::<String>());
    }
}
