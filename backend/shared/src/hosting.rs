//! Who runs this server: chakramcp.com (`managed`) or someone hosting their
//! own network (`self_hosted`, the default), and the settings whose defaults
//! follow from that. Design:
//! `docs/superpowers/specs/2026-10-01-self-hosting-phase3-design.md` §3.
//!
//! | | `self_hosted` | `managed` |
//! |---|---|---|
//! | Credits | off | on |
//! | Public sign-up | closed | open |
//! | `ADMIN_EMAIL` grants admin | no | yes |
//!
//! `CREDITS_ENABLED` and `SIGNUP_ENABLED` override the mode's defaults.

use anyhow::bail;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostingMode {
    #[default]
    SelfHosted,
    Managed,
}

impl HostingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            HostingMode::SelfHosted => "self_hosted",
            HostingMode::Managed => "managed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostingSettings {
    pub mode: HostingMode,
    /// New accounts through the public paths: `POST /v1/auth/signup` and
    /// new users through `/v1/users/upsert`. The operator command always
    /// works.
    pub signup_enabled: bool,
    /// Charging, monthly grants and blocking. Off, the worker discards
    /// queued charges and nobody is blocked for credits; rate limits stay.
    pub credits_enabled: bool,
}

impl Default for HostingSettings {
    fn default() -> Self {
        Self::for_mode(HostingMode::SelfHosted)
    }
}

/// The `server.toml` keys, for `chakramcp-server` (the standalone binaries
/// read the environment only).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HostingFile {
    pub hosting_mode: Option<String>,
    pub signup_enabled: Option<bool>,
    pub credits_enabled: Option<bool>,
}

impl HostingSettings {
    /// The mode's defaults, with no overrides.
    pub fn for_mode(mode: HostingMode) -> Self {
        let managed = mode == HostingMode::Managed;
        Self {
            mode,
            signup_enabled: managed,
            credits_enabled: managed,
        }
    }

    pub fn is_managed(&self) -> bool {
        self.mode == HostingMode::Managed
    }

    /// From the environment only (the standalone binaries).
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok(), &HostingFile::default())
    }

    /// Each setting from a non-empty environment value, else the config
    /// file, else the mode's default. An empty value counts as unset, since
    /// Compose passes one for every `.env` key left out. A value that can't
    /// be parsed stops startup.
    pub fn from_sources(
        env: impl Fn(&str) -> Option<String>,
        file: &HostingFile,
    ) -> anyhow::Result<Self> {
        let mode = match env_value(&env, "HOSTING_MODE").or_else(|| file.hosting_mode.clone()) {
            Some(raw) => parse_mode(&raw)?,
            None => HostingMode::default(),
        };
        let defaults = Self::for_mode(mode);
        let flag = |key: &str, from_file: Option<bool>, default: bool| match env_value(&env, key) {
            Some(raw) => parse_bool(key, &raw),
            None => Ok(from_file.unwrap_or(default)),
        };
        Ok(Self {
            mode,
            signup_enabled: flag(
                "SIGNUP_ENABLED",
                file.signup_enabled,
                defaults.signup_enabled,
            )?,
            credits_enabled: flag(
                "CREDITS_ENABLED",
                file.credits_enabled,
                defaults.credits_enabled,
            )?,
        })
    }

    /// The startup log line, e.g. `hosting_mode=self_hosted credits=off signup=closed`.
    pub fn summary(&self) -> String {
        format!(
            "hosting_mode={} credits={} signup={}",
            self.mode.as_str(),
            if self.credits_enabled { "on" } else { "off" },
            if self.signup_enabled {
                "open"
            } else {
                "closed"
            },
        )
    }
}

/// An environment value, trimmed; `None` when unset or empty.
pub fn env_value(get: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    get(key)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// A strict boolean: `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`, in
/// any case. Anything else is an error naming the key.
pub fn parse_bool(key: &str, raw: &str) -> anyhow::Result<bool> {
    match raw.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("{key}={raw:?} isn't true or false"),
    }
}

fn parse_mode(raw: &str) -> anyhow::Result<HostingMode> {
    match raw.trim().to_lowercase().as_str() {
        "self_hosted" => Ok(HostingMode::SelfHosted),
        "managed" => Ok(HostingMode::Managed),
        _ => bail!("HOSTING_MODE={raw:?} must be self_hosted or managed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(env: &[(&str, &str)], file: HostingFile) -> anyhow::Result<HostingSettings> {
        HostingSettings::from_sources(
            |key| {
                env.iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
            },
            &file,
        )
    }

    #[test]
    fn self_hosted_by_default_with_credits_off_and_signup_closed() {
        let s = settings(&[], HostingFile::default()).unwrap();
        assert_eq!(s, HostingSettings::default());
        assert_eq!(s.mode, HostingMode::SelfHosted);
        assert!(!s.signup_enabled && !s.credits_enabled);
        assert_eq!(
            s.summary(),
            "hosting_mode=self_hosted credits=off signup=closed"
        );
    }

    #[test]
    fn managed_turns_both_on() {
        let s = settings(&[("HOSTING_MODE", " Managed ")], HostingFile::default()).unwrap();
        assert!(s.is_managed());
        assert!(s.signup_enabled && s.credits_enabled);
        assert_eq!(s.summary(), "hosting_mode=managed credits=on signup=open");
    }

    #[test]
    fn overrides_beat_the_mode() {
        let s = settings(
            &[("SIGNUP_ENABLED", "yes"), ("CREDITS_ENABLED", "1")],
            HostingFile::default(),
        )
        .unwrap();
        assert_eq!(s.mode, HostingMode::SelfHosted);
        assert!(s.signup_enabled && s.credits_enabled);

        let s = settings(
            &[("HOSTING_MODE", "managed"), ("SIGNUP_ENABLED", "OFF")],
            HostingFile::default(),
        )
        .unwrap();
        assert!(!s.signup_enabled && s.credits_enabled);
    }

    #[test]
    fn empty_env_values_fall_through_to_the_file() {
        let file = HostingFile {
            hosting_mode: Some("managed".into()),
            signup_enabled: Some(false),
            credits_enabled: None,
        };
        let s = settings(
            &[
                ("HOSTING_MODE", ""),
                ("SIGNUP_ENABLED", "  "),
                ("CREDITS_ENABLED", ""),
            ],
            file.clone(),
        )
        .unwrap();
        assert_eq!(s.mode, HostingMode::Managed);
        assert!(!s.signup_enabled, "from the file");
        assert!(s.credits_enabled, "the mode's default");

        // A set environment value wins over the file.
        let s = settings(&[("HOSTING_MODE", "self_hosted")], file).unwrap();
        assert_eq!(s.mode, HostingMode::SelfHosted);
        assert!(!s.credits_enabled);
    }

    #[test]
    fn bad_values_stop_startup() {
        for (key, value) in [
            ("HOSTING_MODE", "hosted"),
            ("HOSTING_MODE", "self-hosted"),
            ("SIGNUP_ENABLED", "maybe"),
            ("CREDITS_ENABLED", "2"),
        ] {
            let err = settings(&[(key, value)], HostingFile::default()).unwrap_err();
            assert!(err.to_string().contains(key), "{key}={value}: {err}");
        }
        let file = HostingFile {
            hosting_mode: Some("cloud".into()),
            ..Default::default()
        };
        assert!(settings(&[], file).is_err());
    }
}
