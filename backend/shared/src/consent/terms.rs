//! Consent terms (spec §2): what a requester proposes, and what an approver
//! accepts or narrows. Modelled on India's Account Aggregator consent: the
//! requester proposes, the approver accepts, narrows or refuses, and can
//! never widen.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Longest purpose a requester may state.
pub const PURPOSE_MAX_CHARS: usize = 500;
/// Longest validity, in hours (`<n>h`) and in days (`<n>d`).
pub const MAX_HOURS: i64 = 720;
pub const MAX_DAYS: i64 = 30;
/// Most requests one consent may cover.
pub const MAX_REQUESTS: i32 = 10_000;

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// How long a consent lasts: this one request only, or a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Validity {
    #[default]
    Once,
    /// A whole number of hours, in seconds.
    Secs(i64),
}

impl Validity {
    /// From a stored number of seconds; 0 (or less) is a single request.
    pub fn from_secs(secs: i32) -> Self {
        if secs > 0 {
            Validity::Secs(i64::from(secs))
        } else {
            Validity::Once
        }
    }

    /// Seconds a standing consent lasts (0 for a single request).
    pub fn secs(self) -> i64 {
        match self {
            Validity::Once => 0,
            Validity::Secs(s) => s,
        }
    }

    /// Parse `once`, `<n>h` or `<n>d`.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw == "once" {
            return Ok(Validity::Once);
        }
        let bad = || format!("valid_for must be \"once\", \"<n>h\" or \"<n>d\", not {raw:?}");
        let (num, unit) = raw.split_at(raw.len().saturating_sub(1));
        let n: i64 = num.parse().map_err(|_| bad())?;
        match unit {
            "h" if (1..=MAX_HOURS).contains(&n) => Ok(Validity::Secs(n * HOUR)),
            "d" if (1..=MAX_DAYS).contains(&n) => Ok(Validity::Secs(n * DAY)),
            "h" => Err(format!("valid_for hours must be 1 to {MAX_HOURS}")),
            "d" => Err(format!("valid_for days must be 1 to {MAX_DAYS}")),
            _ => Err(bad()),
        }
    }
}

impl fmt::Display for Validity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Validity::Once => f.write_str("once"),
            Validity::Secs(s) if s % DAY == 0 => write!(f, "{}d", s / DAY),
            Validity::Secs(s) => write!(f, "{}h", s / HOUR),
        }
    }
}

impl Serialize for Validity {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Validity {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Validity::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The terms on a consent request. `max_requests` only means something for
/// a standing consent (`valid_for` other than `once`); `None` is unlimited
/// within the window.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConsentTerms {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default)]
    pub valid_for: Validity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_requests: Option<i32>,
}

impl ConsentTerms {
    /// Check what serde can't: lengths and ranges.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(p) = &self.purpose {
            if p.chars().count() > PURPOSE_MAX_CHARS {
                return Err(format!(
                    "purpose must be at most {PURPOSE_MAX_CHARS} characters"
                ));
            }
        }
        if let Some(n) = self.max_requests {
            if !(1..=MAX_REQUESTS).contains(&n) {
                return Err(format!("max_requests must be 1 to {MAX_REQUESTS}"));
            }
        }
        Ok(())
    }

    /// Cap proposed terms at the publisher's longest consent. With a limit
    /// of 0, every request stands alone.
    pub fn capped(mut self, max_validity_secs: i32) -> Self {
        let max = i64::from(max_validity_secs);
        self.valid_for = match self.valid_for {
            Validity::Secs(s) if max > 0 => Validity::Secs(s.min(max)),
            _ => Validity::Once,
        };
        self.normalized()
    }

    /// Whether these terms create a standing consent.
    pub fn is_standing(&self) -> bool {
        self.valid_for != Validity::Once
    }

    /// A single request has no request limit.
    fn normalized(mut self) -> Self {
        if self.valid_for == Validity::Once {
            self.max_requests = None;
        }
        self
    }

    /// The terms an approver settles on: the request's, narrowed by what
    /// they chose. The purpose is the requester's and can't be changed.
    pub fn narrowed(&self, valid_for: Option<Validity>, max_requests: Option<i32>) -> Self {
        Self {
            purpose: self.purpose.clone(),
            valid_for: valid_for.unwrap_or(self.valid_for),
            max_requests: max_requests.or(self.max_requests),
        }
        .normalized()
    }

    /// In words, for approval pages: "Just this request", "For 7 days",
    /// "For 7 days, up to 5 requests".
    pub fn describe(&self) -> String {
        let window = match self.valid_for {
            Validity::Once => return "Just this request".to_owned(),
            Validity::Secs(s) => describe_window(s),
        };
        match self.max_requests {
            Some(1) => format!("For {window}, 1 request"),
            Some(n) => format!("For {window}, up to {n} requests"),
            None => format!("For {window}"),
        }
    }

    /// Whether `self` (approved) stays within `requested` and the
    /// publisher's limit: never longer, never more requests, same purpose.
    pub fn within(&self, requested: &ConsentTerms, max_validity_secs: i32) -> bool {
        let secs = self.valid_for.secs();
        if self.purpose != requested.purpose
            || secs > requested.valid_for.secs()
            || secs > i64::from(max_validity_secs)
        {
            return false;
        }
        if !self.is_standing() {
            return self.max_requests.is_none();
        }
        match (self.max_requests, requested.max_requests) {
            (_, None) => true,
            (Some(approved), Some(asked)) => approved <= asked,
            (None, Some(_)) => false,
        }
    }
}

/// "1 hour", "36 hours", "1 day", "7 days".
fn describe_window(secs: i64) -> String {
    let (n, unit) = if secs % DAY == 0 {
        (secs / DAY, "day")
    } else {
        (secs / HOUR, "hour")
    };
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// The validities an approver may choose for a request asking `requested`
/// on a capability allowing at most `max_validity_secs`, longest first: the
/// request itself, the standard shorter windows, then "once".
pub fn narrowing_options(requested: Validity, max_validity_secs: i32) -> Vec<Validity> {
    let ceiling = requested.secs().min(i64::from(max_validity_secs));
    let mut options: Vec<Validity> = Vec::new();
    if ceiling > 0 {
        options.push(Validity::Secs(ceiling));
        for secs in [30 * DAY, 7 * DAY, DAY, HOUR] {
            if secs < ceiling {
                options.push(Validity::Secs(secs));
            }
        }
    }
    options.push(Validity::Once);
    options
}

impl Validity {
    /// How long, for a choice on an approval page.
    pub fn label(self) -> String {
        match self {
            Validity::Once => "Just this request".to_owned(),
            Validity::Secs(s) => describe_window(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(valid_for: &str, max_requests: Option<i32>) -> ConsentTerms {
        ConsentTerms {
            purpose: Some("book my calendar".into()),
            valid_for: Validity::parse(valid_for).unwrap(),
            max_requests,
        }
    }

    #[test]
    fn validity_parses_and_prints() {
        assert_eq!(Validity::parse("once"), Ok(Validity::Once));
        assert_eq!(Validity::parse("1h"), Ok(Validity::Secs(3600)));
        assert_eq!(Validity::parse("7d"), Ok(Validity::Secs(7 * 86400)));
        assert_eq!(Validity::parse("720h"), Ok(Validity::Secs(720 * 3600)));
        for bad in ["", "d", "0h", "721h", "31d", "1w", "-1d", "1.5h", "forever"] {
            assert!(Validity::parse(bad).is_err(), "{bad} should be refused");
        }
        assert_eq!(Validity::Secs(7 * 86400).to_string(), "7d");
        assert_eq!(Validity::Secs(36 * 3600).to_string(), "36h");
        assert_eq!(Validity::Once.to_string(), "once");
    }

    #[test]
    fn terms_round_trip_as_json() {
        let t: ConsentTerms =
            serde_json::from_value(serde_json::json!({"valid_for": "7d", "max_requests": 5}))
                .unwrap();
        assert_eq!(t.valid_for, Validity::Secs(7 * 86400));
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            serde_json::json!({"valid_for": "7d", "max_requests": 5})
        );
        let empty: ConsentTerms = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(empty.valid_for, Validity::Once);
        assert!(
            serde_json::from_value::<ConsentTerms>(serde_json::json!({"valid_for": "9y"})).is_err()
        );
    }

    #[test]
    fn validate_checks_lengths_and_ranges() {
        assert!(terms("7d", Some(10)).validate().is_ok());
        assert!(terms("7d", Some(0)).validate().is_err());
        assert!(terms("7d", Some(MAX_REQUESTS + 1)).validate().is_err());
        let long = ConsentTerms {
            purpose: Some("x".repeat(PURPOSE_MAX_CHARS + 1)),
            ..ConsentTerms::default()
        };
        assert!(long.validate().is_err());
    }

    #[test]
    fn capped_never_exceeds_the_publishers_limit() {
        assert_eq!(
            terms("30d", None).capped(7 * 86400).valid_for,
            Validity::Secs(7 * 86400)
        );
        assert_eq!(
            terms("1h", None).capped(7 * 86400).valid_for,
            Validity::Secs(3600)
        );
        let single = terms("30d", Some(5)).capped(0);
        assert_eq!(single.valid_for, Validity::Once);
        assert_eq!(single.max_requests, None);
    }

    #[test]
    fn describes_terms_in_words() {
        assert_eq!(terms("once", None).describe(), "Just this request");
        assert_eq!(terms("7d", None).describe(), "For 7 days");
        assert_eq!(
            terms("1d", Some(5)).describe(),
            "For 1 day, up to 5 requests"
        );
        assert_eq!(terms("36h", Some(1)).describe(), "For 36 hours, 1 request");
    }

    #[test]
    fn narrowing_options_never_exceed_the_request_or_the_limit() {
        let d: i32 = 86400;
        let opts = |asked: &str, max: i32| {
            narrowing_options(Validity::parse(asked).unwrap(), max)
                .into_iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(opts("30d", 7 * d), ["7d", "1d", "1h", "once"]);
        assert_eq!(opts("36h", 30 * d), ["36h", "1d", "1h", "once"]);
        assert_eq!(opts("7d", 30 * d), ["7d", "1d", "1h", "once"]);
        assert_eq!(opts("once", 30 * d), ["once"]);
        assert_eq!(opts("7d", 0), ["once"]);
        assert_eq!(Validity::Secs(3600).label(), "1 hour");
    }

    #[test]
    fn within_allows_narrowing_only() {
        let asked = terms("7d", Some(10));
        let max = 30 * 86400;
        assert!(asked.within(&asked, max));
        assert!(asked
            .narrowed(Some(Validity::Secs(86400)), Some(3))
            .within(&asked, max));
        assert!(asked
            .narrowed(Some(Validity::Once), None)
            .within(&asked, max));
        // Longer, more requests, unlimited, or over the publisher's limit.
        assert!(!asked
            .narrowed(Some(Validity::Secs(8 * 86400)), None)
            .within(&asked, max));
        assert!(!asked.narrowed(None, Some(11)).within(&asked, max));
        let mut unlimited = asked.clone();
        unlimited.max_requests = None;
        assert!(!unlimited.within(&asked, max));
        assert!(!asked.within(&asked, 86400));
        // An unlimited request may be approved with a limit.
        let open = terms("7d", None);
        assert!(open.narrowed(None, Some(4)).within(&open, max));
        // The purpose is the requester's.
        let mut reworded = asked.clone();
        reworded.purpose = Some("something else".into());
        assert!(!reworded.within(&asked, max));
        // A single request can't become a standing consent.
        let once = terms("once", None);
        assert!(!once
            .narrowed(Some(Validity::Secs(3600)), None)
            .within(&once, max));
    }
}
