//! Step 4 (spec §4): contact candidates — identifiers a party could be
//! reached at. Tuned for recall: Jev judges each candidate against the
//! capability (S2), so a false candidate costs a question, not a redaction.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::decode::Decoded;
use super::normalize::{normalize, Normalized};
use super::walk::{At, Item};
use super::{CANDIDATE_BYTES, CONTEXT_BYTES};

/// The contact kinds (a subset of [`super::FindingKind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContactKind {
    Email,
    Phone,
    Url,
    MessagingLink,
    Handle,
}

impl ContactKind {
    pub fn finding(self) -> super::FindingKind {
        use super::FindingKind as K;
        match self {
            ContactKind::Email => K::Email,
            ContactKind::Phone => K::Phone,
            ContactKind::Url => K::Url,
            ContactKind::MessagingLink => K::MessagingLink,
            ContactKind::Handle => K::Handle,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.finding().as_str()
    }
}

/// Where a candidate was found.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// In an item's original text, at this byte range.
    Item { item: usize, range: Range<usize> },
    /// In a decoded span (redaction replaces the whole encoded span).
    Decoded { decoded: usize },
}

/// A contact candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: usize,
    pub kind: ContactKind,
    pub source: Source,
    /// The normalized match, at most [`CANDIDATE_BYTES`].
    pub text: String,
    /// Normalized text around it, at most [`CONTEXT_BYTES`].
    pub context: String,
}

impl Candidate {
    /// The item the candidate belongs to.
    pub fn item(&self, decoded: &[Decoded]) -> usize {
        match &self.source {
            Source::Item { item, .. } => *item,
            Source::Decoded { decoded: d } => decoded[*d].item,
        }
    }
}

/// Find candidates in the items and the decoded spans, numbered in order.
pub fn find(items: &[Item], decoded: &[Decoded]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let n = normalize(&item.text);
        let found = if item.at == At::Num {
            phone_number_item(&n.text)
        } else {
            find_in(&n.text)
        };
        for (range, kind) in found {
            out.push(Candidate {
                id: 0,
                kind,
                source: Source::Item {
                    item: i,
                    range: n.original(range.clone()),
                },
                text: cut(&n.text[range.clone()], CANDIDATE_BYTES),
                context: context(&n, range),
            });
        }
    }
    for (d, dec) in decoded.iter().enumerate() {
        let n = normalize(&dec.text);
        for (range, kind) in find_in(&n.text) {
            out.push(Candidate {
                id: 0,
                kind,
                source: Source::Decoded { decoded: d },
                text: cut(&n.text[range.clone()], CANDIDATE_BYTES),
                context: context(&n, range),
            });
        }
    }
    for (i, c) in out.iter_mut().enumerate() {
        c.id = i;
    }
    out
}

/// Candidates in one normalized text: the longest match wins an overlap.
pub fn find_in(text: &str) -> Vec<(Range<usize>, ContactKind)> {
    let excluded = exclusions(text);
    let mut all: Vec<(Range<usize>, ContactKind)> = Vec::new();
    all.extend(emails(text));
    all.extend(urls(text));
    all.extend(handles(text));
    all.extend(
        phones(text)
            .into_iter()
            .filter(|(r, _)| !excluded.iter().any(|x| overlaps(x, r))),
    );
    all.extend(digit_words(text));
    all.sort_by_key(|(r, _)| std::cmp::Reverse(r.len()));
    let mut kept: Vec<(Range<usize>, ContactKind)> = Vec::new();
    for (r, k) in all {
        if !kept.iter().any(|(k_r, _)| overlaps(k_r, &r)) {
            kept.push((r, k));
        }
    }
    kept.sort_by_key(|(r, _)| r.start);
    kept
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// No letter, digit or `_` directly before `start` or after `end`.
fn bounded(text: &str, r: &Range<usize>) -> bool {
    let before = text[..r.start].chars().next_back();
    let after = text[r.end..].chars().next();
    let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
    edge(before) && edge(after)
}

// ─── Emails ──────────────────────────────────────────────

/// `@`, or `at` spelled out (bare, or in brackets).
const AT: &str = r"(?:\s*@\s*|\s*[\(\[\{<]\s*at\s*[\)\]\}>]\s*|\s+at\s+)";
/// `.` (spacing allowed), or `dot` spelled out (bare, or in brackets).
const DOT: &str = r"(?:\s*\.\s*|\s*[\(\[\{<]\s*dot\s*[\)\]\}>]\s*|\s+dot\s+)";

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"[a-z0-9][a-z0-9._%+\-]*{AT}[a-z0-9][a-z0-9\-]*(?:{DOT}[a-z0-9][a-z0-9\-]*)*{DOT}[a-z]{{2,24}}"
    ))
    .expect("email regex")
});

fn emails(text: &str) -> Vec<(Range<usize>, ContactKind)> {
    EMAIL
        .find_iter(text)
        .map(|m| m.range())
        .filter(|r| bounded(text, r))
        .filter(|r| {
            let s = &text[r.clone()];
            if !s.contains(char::is_whitespace) {
                return true;
            }
            // Spaced or spelled-out forms need a known TLD, and a spelled
            // `at` with a plain full stop is prose ("look at this. it").
            let prose = !s.contains('@') && (s.contains(". ") || s.contains(" ."));
            TLDS.contains(&last_label(s)) && !prose
        })
        .map(|r| (r, ContactKind::Email))
        .collect()
}

fn last_label(s: &str) -> &str {
    s.rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .next()
        .unwrap_or("")
}

// ─── URLs and messaging links ────────────────────────────

/// TLDs a bare domain may end in. Real TLDs that are also common file
/// extensions (`.py`, `.rs`, `.md`, `.sh`, `.ts`, `.pl`, `.so`, `.ps`) are
/// left out, so `main.py` isn't a link; with a scheme they still match.
const TLDS: &[&str] = &[
    "com", "net", "org", "io", "in", "co", "me", "ai", "app", "dev", "info", "biz", "xyz", "us",
    "uk", "ca", "au", "de", "fr", "jp", "cn", "ru", "br", "es", "it", "nl", "se", "no", "fi", "dk",
    "ch", "at", "be", "ie", "nz", "sg", "hk", "tw", "kr", "id", "my", "ph", "vn", "th", "pk", "bd",
    "lk", "np", "ae", "sa", "za", "ng", "ke", "eg", "tr", "il", "gr", "pt", "cz", "hu", "ro", "ua",
    "edu", "gov", "tv", "cc", "ly", "gg", "gl", "to", "link", "site", "online", "store", "tech",
    "cloud", "page", "live", "blog", "shop", "club", "top", "vip", "pro", "mobi", "name", "email",
    "chat", "social", "world", "space", "fun", "one", "life",
];

/// Hosts (and path prefixes) of messaging and social profiles.
const MESSAGING: &[&str] = &[
    "t.me",
    "telegram.me",
    "telegram.dog",
    "wa.me",
    "api.whatsapp.com",
    "chat.whatsapp.com",
    "discord.gg",
    "discord.com/invite",
    "discord.com/users",
    "signal.me",
    "signal.group",
    "linkedin.com/in",
    "instagram.com",
    "x.com",
    "twitter.com",
    "facebook.com",
    "fb.me",
    "m.me",
    "snapchat.com/add",
    "line.me",
    "u.wechat.com",
    "threads.net",
    "tiktok.com",
    "skype.com",
];

static URL_SCHEME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:https?|ftp|tg|whatsapp|skype|viber|signal)://[^\s<>"'`]+"#).expect("url regex")
});

static BARE_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    let tlds = TLDS.join("|");
    Regex::new(&format!(
        r#"(?:[a-z0-9](?:[a-z0-9\-]{{0,61}}[a-z0-9])?\.)+(?:{tlds})(?::\d{{2,5}})?(?:/[^\s<>"'`]*)?"#
    ))
    .expect("domain regex")
});

fn urls(text: &str) -> Vec<(Range<usize>, ContactKind)> {
    let mut out: Vec<Range<usize>> = URL_SCHEME.find_iter(text).map(|m| m.range()).collect();
    for m in BARE_DOMAIN.find_iter(text) {
        let r = m.range();
        // A domain right after `@` belongs to an email or handle, and one
        // glued to a word isn't a domain.
        let before = text[..r.start].chars().next_back();
        if before.is_some_and(|c| c == '@' || c.is_alphanumeric() || c == '_') {
            continue;
        }
        if !out.iter().any(|o| overlaps(o, &r)) {
            out.push(r);
        }
    }
    out.into_iter()
        .map(|mut r| {
            // Trailing punctuation ends a sentence, not a link.
            while text[..r.end].ends_with(['.', ',', ';', ':', '!', '?', ')', ']']) {
                r.end -= 1;
            }
            let kind = if messaging(&text[r.clone()]) {
                ContactKind::MessagingLink
            } else {
                ContactKind::Url
            };
            (r, kind)
        })
        .filter(|(r, _)| !r.is_empty())
        .collect()
}

fn messaging(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    MESSAGING.iter().any(|m| {
        rest.strip_prefix(m)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with(['/', '?', '#', ':']))
    })
}

// ─── Handles ─────────────────────────────────────────────

static HANDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@[a-z0-9_.]{3,30}").expect("handle regex"));

fn handles(text: &str) -> Vec<(Range<usize>, ContactKind)> {
    HANDLE
        .find_iter(text)
        .filter_map(|m| {
            let mut r = m.range();
            // Not the domain half of an email.
            let before = text[..r.start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || "._%+-".contains(c)) {
                return None;
            }
            while text[..r.end].ends_with('.') {
                r.end -= 1;
            }
            (r.len() > 3).then_some((r, ContactKind::Handle))
        })
        .collect()
}

// ─── Phones ──────────────────────────────────────────────

static E164: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\+\d[\d\s\-\.\(\)]{6,22}\d").expect("e164 regex"));
static INDIAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\+?91[\s\-]?|0)?[6-9](?:[\s\-]?\d){9}").expect("indian regex")
});
static SEPARATED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(?\d[\d\s\-\.\(\)]{8,22}\d").expect("separated regex"));

fn digits(s: &str) -> usize {
    s.chars().filter(char::is_ascii_digit).count()
}

fn phones(text: &str) -> Vec<(Range<usize>, ContactKind)> {
    let mut out = Vec::new();
    for m in E164.find_iter(text) {
        let r = m.range();
        if (8..=15).contains(&digits(m.as_str())) && bounded(text, &r) {
            out.push(r);
        }
    }
    for m in INDIAN.find_iter(text) {
        let r = m.range();
        if bounded(text, &r) {
            out.push(r);
        }
    }
    for m in SEPARATED.find_iter(text) {
        let s = m.as_str();
        let separated = s.contains([' ', '-', '.', '(', ')']);
        if separated && (10..=15).contains(&digits(s)) && bounded(text, &m.range()) {
            out.push(m.range());
        }
    }
    out.into_iter().map(|r| (r, ContactKind::Phone)).collect()
}

/// A walked number is a phone when its digits are an Indian mobile or a
/// 10–15 digit number.
fn phone_number_item(digits_text: &str) -> Vec<(Range<usize>, ContactKind)> {
    let d = digits_text;
    let indian = |s: &str| s.len() == 10 && s.starts_with(['6', '7', '8', '9']);
    let phone = indian(d)
        || d.strip_prefix("91").is_some_and(indian)
        || d.strip_prefix('0').is_some_and(indian)
        || (10..=15).contains(&d.len());
    if phone {
        vec![(0..d.len(), ContactKind::Phone)]
    } else {
        Vec::new()
    }
}

/// Spans that look like numbers but aren't phones: dates, times, amounts,
/// versions, UUIDs, and digits inside longer tokens.
fn exclusions(text: &str) -> Vec<Range<usize>> {
    static NOT_PHONES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"\d{4}-\d{2}-\d{2}(?:[t\s]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:z|[+\-]\d{2}:?\d{2})?)?",
            r"|\d{1,2}[/\-\.]\d{1,2}[/\-\.]\d{2,4}",
            r"|\d{1,2}:\d{2}(?::\d{2})?",
            r"|[$€£₹¥]\s?\d[\d,]*(?:\.\d+)?|\d[\d,]*\.\d{2}\b",
            r"|v?\d+\.\d+\.\d+(?:[\-+][0-9a-z.\-]+)?",
            r"|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
            r"|[a-z_]+\d[a-z0-9_]*|\d+[a-z_][a-z0-9_]*",
        ))
        .expect("exclusion regex")
    });
    NOT_PHONES.find_iter(text).map(|m| m.range()).collect()
}

// ─── Digits written as words ─────────────────────────────

static DIGIT_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:zero|oh|one|two|three|four|five|six|seven|eight|nine|double|triple)\b[\s,\-]*){7,}")
        .expect("digit words regex")
});

fn digit_words(text: &str) -> Vec<(Range<usize>, ContactKind)> {
    DIGIT_WORDS
        .find_iter(text)
        .filter_map(|m| {
            let mut count = 0;
            let mut repeat = 1;
            for w in m
                .as_str()
                .split(|c: char| !c.is_ascii_alphabetic())
                .filter(|w| !w.is_empty())
            {
                match w {
                    "double" => repeat = 2,
                    "triple" => repeat = 3,
                    _ => {
                        count += repeat;
                        repeat = 1;
                    }
                }
            }
            let r = m.range();
            let trimmed = r.start..r.start + m.as_str().trim_end_matches([' ', ',', '-']).len();
            (count >= 7).then_some((trimmed, ContactKind::Phone))
        })
        .collect()
}

// ─── Snippets ────────────────────────────────────────────

/// At most `max` bytes of `s`, cut on a character boundary.
pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// The match with text on both sides, at most [`CONTEXT_BYTES`] in all.
fn context(n: &Normalized, range: Range<usize>) -> String {
    let text = &n.text;
    let room = CONTEXT_BYTES.saturating_sub(range.len().min(CANDIDATE_BYTES)) / 2;
    let mut start = range.start.saturating_sub(room);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (range.end + room).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    cut(&text[start..end], CONTEXT_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(String, ContactKind)> {
        let n = normalize(text).text;
        find_in(&n)
            .into_iter()
            .map(|(r, k)| (n[r].to_owned(), k))
            .collect()
    }

    fn one(text: &str) -> (String, ContactKind) {
        let found = kinds(text);
        assert_eq!(found.len(), 1, "{text}: {found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn finds_emails_plain_and_spelled_out() {
        assert_eq!(
            one("write to john.doe+x@example.co.in now").0,
            "john.doe+x@example.co.in"
        );
        for spelled in [
            "john at example dot com",
            "john [at] example [dot] com",
            "john(at)example(dot)com",
            "john @ example . com",
        ] {
            let (_, k) = one(&format!("mail {spelled} please"));
            assert_eq!(k, ContactKind::Email, "{spelled}");
        }
        // Prose with "at" and a full stop isn't an address.
        assert!(kinds("look at this. it works").is_empty());
        assert!(kinds("meet at the cafe dot later").is_empty());
    }

    #[test]
    fn finds_phones() {
        for phone in [
            "+91 98450 12345",
            "+1 (415) 555-0134",
            "+44 20 7946 0958",
            "98450 12345",
            "9845012345",
            "091-98450-12345",
            "415-555-0134 ext",
        ] {
            let found = kinds(&format!("call {phone}"));
            assert!(
                found.iter().any(|(_, k)| *k == ContactKind::Phone),
                "{phone}: {found:?}"
            );
        }
        let (text, kind) = one("my number is nine eight four five zero one two three four five");
        assert_eq!(kind, ContactKind::Phone);
        assert!(text.starts_with("nine") && text.ends_with("five"));
        assert_eq!(
            one("double five one two three four five six").1,
            ContactKind::Phone
        );
    }

    #[test]
    fn leaves_numbers_that_arent_phones() {
        for not_phone in [
            "2026-10-23",
            "2026-10-23T10:30:00Z",
            "23/10/2026",
            "₹1,999.00",
            "$12,345,678.90",
            "v2.3.1",
            "10.4.12",
            "9b2e6c1e-3f4a-4c2b-9d1e-1234567890ab",
            "order ord1234567890",
            "build 20261023",
            "one two three",
        ] {
            assert!(
                !kinds(not_phone)
                    .iter()
                    .any(|(_, k)| *k == ContactKind::Phone),
                "{not_phone}: {:?}",
                kinds(not_phone)
            );
        }
    }

    #[test]
    fn finds_links_and_tells_messaging_apart() {
        assert_eq!(
            one("see https://example.com/a?b=1.").0,
            "https://example.com/a?b=1"
        );
        assert_eq!(one("visit www.example.org today").1, ContactKind::Url);
        for link in [
            "t.me/acme_sales",
            "https://wa.me/919845012345",
            "discord.gg/abc123",
            "https://www.linkedin.com/in/jane-doe",
            "instagram.com/acme.shop",
            "x.com/acme",
        ] {
            assert_eq!(
                one(&format!("ping me: {link}")).1,
                ContactKind::MessagingLink,
                "{link}"
            );
        }
        // File names aren't domains.
        for not_link in [
            "open file.txt",
            "built on Node.js",
            "run main.py",
            "see report.pdf",
        ] {
            assert!(
                kinds(not_link).is_empty(),
                "{not_link}: {:?}",
                kinds(not_link)
            );
        }
    }

    #[test]
    fn finds_handles_but_not_inside_emails() {
        assert_eq!(one("dm @acme_sales_bot.").0, "@acme_sales_bot");
        assert_eq!(one("john@example.com").1, ContactKind::Email);
        // App names alone aren't candidates.
        assert!(kinds("let's move this to Telegram or WhatsApp").is_empty());
    }

    #[test]
    fn the_longest_match_wins() {
        let found = kinds("mail sales@example.com");
        assert_eq!(
            found,
            vec![("sales@example.com".to_owned(), ContactKind::Email)]
        );
    }

    #[test]
    fn numbers_as_items() {
        assert!(!phone_number_item("9845012345").is_empty());
        assert!(!phone_number_item("919845012345").is_empty());
        assert!(!phone_number_item("14155550134").is_empty());
        assert!(phone_number_item("1234567").is_empty());
        assert!(phone_number_item("123456789").is_empty());
    }

    #[test]
    fn snippets_are_bounded() {
        let long = format!("{} john@example.com {}", "x".repeat(500), "y".repeat(500));
        let n = normalize(&long);
        let r = find_in(&n.text)[0].0.clone();
        let c = context(&n, r);
        assert!(c.len() <= CONTEXT_BYTES);
        assert!(c.contains("john@example.com"));
        assert!(cut(&"é".repeat(100), CANDIDATE_BYTES).len() <= CANDIDATE_BYTES);
    }
}
