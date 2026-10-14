//! Whether this server sells credits, and on what terms (spec §5.1, §9).
//!
//! Purchasing is on only on a `managed` server (chakramcp.com) with credits
//! on and all four Dodo settings present. A problem in the settings turns
//! purchasing off with an ERROR, never stops the server: the same process
//! runs the relay, and a half-edited `.env` mustn't take invocations down.
//! `chakramcp_purchase_config_error` makes such a problem alertable.

use serde::Serialize;

use super::dodo::{DodoClient, Secret, WebhookKey, LIVE_BASE_URL, TEST_BASE_URL};
use crate::credits_service::MAX_AMOUNT_MC;

pub const DEFAULT_CREDITS_PER_USD: i64 = 1_000;
pub const DEFAULT_MIN_CENTS: i64 = 100;
pub const DEFAULT_MAX_CENTS: i64 = 500_000;

const DODO_KEYS: [&str; 4] = [
    "DODO_PAYMENTS_API_KEY",
    "DODO_PAYMENTS_WEBHOOK_KEY",
    "DODO_PAYMENTS_PRODUCT_ID",
    "DODO_PAYMENTS_ENVIRONMENT",
];
const PRICE_KEYS: [&str; 3] = [
    "CREDITS_PURCHASE_PER_USD",
    "CREDITS_PURCHASE_MIN_CENTS",
    "CREDITS_PURCHASE_MAX_CENTS",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Test,
    Live,
}

impl Mode {
    fn base_url(self) -> &'static str {
        match self {
            Mode::Test => TEST_BASE_URL,
            Mode::Live => LIVE_BASE_URL,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Test => "test_mode",
            Mode::Live => "live_mode",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PurchaseConfig {
    pub client: DodoClient,
    pub webhook_key: WebhookKey,
    /// The pay-what-you-want credits product.
    pub product_id: String,
    pub mode: Mode,
    pub credits_per_usd: i64,
    pub min_cents: i32,
    pub max_cents: i32,
}

/// What the web UI needs to offer a purchase (`CreditsView.purchase`).
#[derive(Debug, Clone, Serialize)]
pub struct PurchaseInfo {
    pub min_cents: i32,
    pub max_cents: i32,
    pub credits_per_usd: i64,
    pub mode: Mode,
}

impl PurchaseConfig {
    /// Milli-credits bought by `cents`: 1 credit is 1,000 mc and $1 is 100
    /// cents. Within bounds, startup checked this can't overflow.
    pub fn credits_mc(&self, cents: i32) -> i64 {
        i64::from(cents) * self.credits_per_usd * 10
    }

    pub fn info(&self) -> PurchaseInfo {
        PurchaseInfo {
            min_cents: self.min_cents,
            max_cents: self.max_cents,
            credits_per_usd: self.credits_per_usd,
            mode: self.mode,
        }
    }
}

#[derive(Debug)]
pub enum PurchaseSetup {
    On(PurchaseConfig),
    /// `error` is a setting that's present but wrong, as opposed to
    /// purchasing simply not being set up here.
    Off {
        reason: String,
        error: bool,
    },
}

/// Read the purchase settings through `get` (the environment, or a map in
/// tests). Empty values count as unset.
pub fn load(
    get: impl Fn(&str) -> Option<String>,
    managed: bool,
    credits_enabled: bool,
) -> PurchaseSetup {
    let value = |key: &str| {
        get(key)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let present: Vec<&str> = DODO_KEYS
        .iter()
        .chain(PRICE_KEYS.iter())
        .copied()
        .filter(|k| value(k).is_some())
        .collect();
    let off = |reason: &str, error: bool| PurchaseSetup::Off {
        reason: reason.to_owned(),
        error,
    };
    if !managed {
        return off("not chakramcp.com: HOSTING_MODE isn't managed", false);
    }
    if !credits_enabled {
        return off("credits are off", false);
    }
    if !DODO_KEYS.iter().any(|k| value(k).is_some()) {
        let error = !present.is_empty();
        return off("no Dodo settings", error);
    }
    match configure(&value) {
        Ok(config) => PurchaseSetup::On(config),
        Err(reason) => off(&reason, true),
    }
}

fn configure(value: &impl Fn(&str) -> Option<String>) -> Result<PurchaseConfig, String> {
    let missing: Vec<&str> = DODO_KEYS
        .iter()
        .copied()
        .filter(|k| value(k).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(format!("{} missing", missing.join(", ")));
    }
    let get = |k: &str| value(k).expect("checked above");
    let mode = match get("DODO_PAYMENTS_ENVIRONMENT").as_str() {
        "test_mode" => Mode::Test,
        "live_mode" => Mode::Live,
        other => {
            return Err(format!(
                "DODO_PAYMENTS_ENVIRONMENT is {other:?}, not test_mode or live_mode"
            ))
        }
    };
    let webhook_key = WebhookKey::parse(&get("DODO_PAYMENTS_WEBHOOK_KEY"))?;
    let number = |key: &str, default: i64| -> Result<i64, String> {
        match value(key) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("{key} isn't a whole number")),
        }
    };
    let credits_per_usd = number("CREDITS_PURCHASE_PER_USD", DEFAULT_CREDITS_PER_USD)?;
    let min_cents = number("CREDITS_PURCHASE_MIN_CENTS", DEFAULT_MIN_CENTS)?;
    let max_cents = number("CREDITS_PURCHASE_MAX_CENTS", DEFAULT_MAX_CENTS)?;
    if credits_per_usd <= 0 {
        return Err("CREDITS_PURCHASE_PER_USD must be above 0".into());
    }
    if min_cents <= 0 || min_cents > max_cents {
        return Err(
            "the purchase limits need 0 < CREDITS_PURCHASE_MIN_CENTS ≤ CREDITS_PURCHASE_MAX_CENTS"
                .into(),
        );
    }
    let max_cents_i32 = i32::try_from(max_cents)
        .map_err(|_| "CREDITS_PURCHASE_MAX_CENTS is too large".to_owned())?;
    let largest = max_cents
        .checked_mul(credits_per_usd)
        .and_then(|v| v.checked_mul(10));
    if largest.is_none_or(|mc| mc > MAX_AMOUNT_MC) {
        return Err("the largest purchase would exceed a billion credits".into());
    }
    let client = DodoClient::new(mode.base_url(), Secret::new(get("DODO_PAYMENTS_API_KEY")))
        .map_err(|e| format!("couldn't build the Dodo client: {e}"))?;
    Ok(PurchaseConfig {
        client,
        webhook_key,
        product_id: get("DODO_PAYMENTS_PRODUCT_ID"),
        mode,
        credits_per_usd,
        min_cents: i32::try_from(min_cents).expect("min ≤ max, which fits"),
        max_cents: max_cents_i32,
    })
}

/// Load from the environment, log the outcome next to the hosting line, and
/// set the config-error gauge. Both mains call this.
pub fn from_env(managed: bool, credits_enabled: bool) -> Option<PurchaseConfig> {
    match load(|k| std::env::var(k).ok(), managed, credits_enabled) {
        PurchaseSetup::On(config) => {
            chakramcp_shared::telemetry::set_purchase_config_error(false);
            tracing::info!(
                "purchasing=on ({}, ${:.2}–${:.2}, {} credits per dollar)",
                config.mode.label(),
                f64::from(config.min_cents) / 100.0,
                f64::from(config.max_cents) / 100.0,
                config.credits_per_usd
            );
            Some(config)
        }
        PurchaseSetup::Off { reason, error } => {
            chakramcp_shared::telemetry::set_purchase_config_error(error);
            if error {
                tracing::error!("purchasing=off ({reason})");
            } else {
                tracing::info!("purchasing=off ({reason})");
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const KEY: &str = "whsec_c2VjcmV0LWtleS1mb3ItdGVzdHM=";

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn full() -> Vec<(&'static str, &'static str)> {
        vec![
            ("DODO_PAYMENTS_API_KEY", "test-key"),
            ("DODO_PAYMENTS_WEBHOOK_KEY", KEY),
            ("DODO_PAYMENTS_PRODUCT_ID", "pdt_1"),
            ("DODO_PAYMENTS_ENVIRONMENT", "test_mode"),
        ]
    }

    fn reason(setup: PurchaseSetup) -> (String, bool) {
        match setup {
            PurchaseSetup::Off { reason, error } => (reason, error),
            PurchaseSetup::On(_) => panic!("expected purchasing off"),
        }
    }

    #[test]
    fn all_four_settings_turn_it_on_with_the_defaults() {
        let PurchaseSetup::On(cfg) = load(env(&full()), true, true) else {
            panic!("expected purchasing on")
        };
        assert_eq!(cfg.mode, Mode::Test);
        assert_eq!(
            (cfg.min_cents, cfg.max_cents, cfg.credits_per_usd),
            (100, 500_000, 1_000)
        );
        assert_eq!(cfg.credits_mc(1), 10_000, "a cent buys 10 credits");
        assert_eq!(cfg.credits_mc(237), 2_370_000);
        assert_eq!(cfg.credits_mc(500_000), 5_000_000_000);
        assert!(
            !format!("{cfg:?}").contains("test-key"),
            "the key never shows"
        );
    }

    #[test]
    fn nothing_set_is_quietly_off() {
        assert_eq!(
            reason(load(env(&[]), true, true)),
            ("no Dodo settings".into(), false)
        );
        assert!(!reason(load(env(&full()), false, true)).1, "self-hosted");
        assert!(!reason(load(env(&full()), true, false)).1, "credits off");
        let blank: Vec<_> = full().into_iter().map(|(k, _)| (k, "  ")).collect();
        assert!(!reason(load(env(&blank), true, true)).1, "blank is unset");
    }

    #[test]
    fn a_broken_setup_is_off_with_an_error() {
        let mut partial = full();
        partial.retain(|(k, _)| *k != "DODO_PAYMENTS_WEBHOOK_KEY");
        let (why, error) = reason(load(env(&partial), true, true));
        assert!(error && why.contains("DODO_PAYMENTS_WEBHOOK_KEY"), "{why}");

        let only_prices = [("CREDITS_PURCHASE_PER_USD", "500")];
        assert!(
            reason(load(env(&only_prices), true, true)).1,
            "prices without Dodo"
        );

        for (key, value, says) in [
            (
                "DODO_PAYMENTS_ENVIRONMENT",
                "production",
                "test_mode or live_mode",
            ),
            ("DODO_PAYMENTS_WEBHOOK_KEY", "whsec_???", "whsec_"),
            ("CREDITS_PURCHASE_PER_USD", "0", "above 0"),
            ("CREDITS_PURCHASE_PER_USD", "lots", "whole number"),
            ("CREDITS_PURCHASE_MIN_CENTS", "0", "limits"),
            ("CREDITS_PURCHASE_MIN_CENTS", "600000", "limits"),
            ("CREDITS_PURCHASE_MAX_CENTS", "3000000000", "too large"),
            ("CREDITS_PURCHASE_PER_USD", "999999999999", "billion"),
        ] {
            let mut settings = full();
            settings.retain(|(k, _)| *k != key);
            settings.push((key, value));
            let (why, error) = reason(load(env(&settings), true, true));
            assert!(error && why.contains(says), "{key}={value}: {why}");
        }
    }

    #[test]
    fn live_mode_is_recognised() {
        let mut settings = full();
        settings.retain(|(k, _)| *k != "DODO_PAYMENTS_ENVIRONMENT");
        settings.push(("DODO_PAYMENTS_ENVIRONMENT", "live_mode"));
        let PurchaseSetup::On(cfg) = load(env(&settings), true, true) else {
            panic!("expected purchasing on")
        };
        assert_eq!(cfg.mode, Mode::Live);
        assert_eq!(serde_json::to_value(cfg.info()).unwrap()["mode"], "live");
    }
}
