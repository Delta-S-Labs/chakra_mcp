//! Dodo Payments: creating checkout sessions and reading its webhooks.
//! Spec: `docs/superpowers/specs/2026-10-06-credits-p4-purchasing-design.md`
//! §6.1 and §7.
//!
//! Two calls don't justify Dodo's SDK: this is one `POST /checkouts` and the
//! Standard Webhooks signature check Dodo uses.

use std::fmt;
use std::time::Duration;

use axum::http::HeaderMap;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use uuid::Uuid;

pub const TEST_BASE_URL: &str = "https://test.dodopayments.com";
pub const LIVE_BASE_URL: &str = "https://live.dodopayments.com";
/// A checkout session must be created within this, or the buyer gets a 502.
const TIMEOUT: Duration = Duration::from_secs(15);
/// Webhooks stamped further than this from now are refused (Standard
/// Webhooks' replay window).
pub const TOLERANCE_SECS: u64 = 5 * 60;

/// A secret that never shows in logs or `Debug` output.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[derive(Clone, Debug)]
pub struct DodoClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Secret,
}

/// What we ask Dodo for: one credits product at the amount the buyer chose.
#[derive(Debug)]
pub struct NewCheckout<'a> {
    pub product_id: &'a str,
    pub amount_cents: i32,
    pub email: &'a str,
    pub name: Option<&'a str>,
    pub return_url: &'a str,
    pub checkout_id: Uuid,
    pub account_slug: &'a str,
}

impl NewCheckout<'_> {
    /// The request body. Discount codes and currency selection are on by
    /// default in Dodo; both would let a buyer pay something other than
    /// `amount_cents`, so they're turned off.
    pub fn body(&self) -> Value {
        json!({
            "product_cart": [{
                "product_id": self.product_id,
                "quantity": 1,
                "amount": self.amount_cents,
            }],
            "customer": { "email": self.email, "name": self.name },
            "billing_currency": "USD",
            "feature_flags": {
                "allow_discount_code": false,
                "allow_currency_selection": false,
            },
            "return_url": self.return_url,
            "metadata": {
                "checkout_id": self.checkout_id.to_string(),
                "account": self.account_slug,
            },
        })
    }
}

#[derive(Debug, Clone)]
pub struct CreatedSession {
    pub session_id: String,
    pub checkout_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DodoError {
    #[error("request to Dodo failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Dodo answered {status}: {body}")]
    Status { status: u16, body: String },
    #[error("Dodo's answer had no checkout_url")]
    NoCheckoutUrl,
}

impl DodoClient {
    /// `base_url` is [`TEST_BASE_URL`] or [`LIVE_BASE_URL`]; tests pass a
    /// local fake.
    pub fn new(base_url: impl Into<String>, api_key: Secret) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(format!(
                "chakramcp-app/{}",
                chakramcp_shared::telemetry::VERSION
            ))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
        })
    }

    pub async fn create_checkout(
        &self,
        checkout: &NewCheckout<'_>,
    ) -> Result<CreatedSession, DodoError> {
        #[derive(Deserialize)]
        struct Response {
            session_id: String,
            checkout_url: Option<String>,
        }

        let res = self
            .http
            .post(format!("{}/checkouts", self.base_url))
            .bearer_auth(self.api_key.expose())
            .json(&checkout.body())
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            let mut body = res.text().await.unwrap_or_default();
            body.truncate(300);
            return Err(DodoError::Status {
                status: status.as_u16(),
                body,
            });
        }
        let created: Response = res.json().await?;
        Ok(CreatedSession {
            session_id: created.session_id,
            checkout_url: created.checkout_url.ok_or(DodoError::NoCheckoutUrl)?,
        })
    }
}

// ─── Webhooks ────────────────────────────────────────────

/// The signing key, `whsec_` plus base64.
#[derive(Clone)]
pub struct WebhookKey(Vec<u8>);

impl WebhookKey {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let encoded = raw.strip_prefix("whsec_").unwrap_or(raw);
        match B64.decode(encoded) {
            Ok(bytes) if !bytes.is_empty() => Ok(Self(bytes)),
            _ => Err("DODO_PAYMENTS_WEBHOOK_KEY isn't a whsec_ key".into()),
        }
    }
}

impl fmt::Debug for WebhookKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebhookKey(***)")
    }
}

/// Why a webhook was refused; `reason()` is the metric label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    MissingHeader,
    Stale,
    BadSignature,
    BadJson,
}

impl Rejection {
    pub fn reason(self) -> &'static str {
        match self {
            Rejection::MissingHeader => "missing_header",
            Rejection::Stale => "stale",
            Rejection::BadSignature => "bad_signature",
            Rejection::BadJson => "bad_json",
        }
    }
}

/// The Standard Webhooks check: the HMAC-SHA256 of `{id}.{timestamp}.{body}`
/// must match one of the header's space-separated `v1,<base64>` signatures
/// (several allow key rotation), and the timestamp must be within five
/// minutes of `now` (Unix seconds).
pub fn verify_webhook(
    headers: &HeaderMap,
    body: &[u8],
    key: &WebhookKey,
    now: i64,
) -> Result<(), Rejection> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let (Some(id), Some(timestamp), Some(signatures)) = (
        header("webhook-id"),
        header("webhook-timestamp"),
        header("webhook-signature"),
    ) else {
        return Err(Rejection::MissingHeader);
    };
    let sent: i64 = timestamp.parse().map_err(|_| Rejection::Stale)?;
    match now.checked_sub(sent) {
        Some(age) if age.unsigned_abs() <= TOLERANCE_SECS => {}
        _ => return Err(Rejection::Stale),
    }

    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(&key.0).expect("HMAC takes keys of any length");
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = B64.encode(mac.finalize().into_bytes());
    let matched = signatures
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("v1,"))
        .any(|s| bool::from(s.as_bytes().ct_eq(expected.as_bytes())));
    if matched {
        Ok(())
    } else {
        Err(Rejection::BadSignature)
    }
}

/// The parts of Dodo's payment object we use. Only `payment_id` is required:
/// the rest can be `null` in Dodo's payloads.
#[derive(Debug, Clone, Deserialize)]
pub struct Payment {
    pub payment_id: String,
    #[serde(default)]
    pub checkout_session_id: Option<String>,
    /// Including tax, in the currency's smallest unit.
    #[serde(default)]
    pub total_amount: Option<i64>,
    #[serde(default)]
    pub tax: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub discounts: Option<Vec<Value>>,
    #[serde(default)]
    pub product_cart: Option<Vec<CartLine>>,
    #[serde(default)]
    pub invoice_url: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CartLine {
    pub product_id: String,
}

#[derive(Debug)]
pub enum Event {
    PaymentSucceeded(Payment),
    /// `payment.failed` or `payment.cancelled`.
    PaymentFailed(Payment),
    /// `refund.*` and `dispute.*`: logged for a manual adjustment.
    Notice {
        kind: String,
        payment_id: Option<String>,
    },
    /// Everything else, `payment.processing` included.
    Ignored(String),
}

/// Parse a verified webhook body.
pub fn parse_event(body: &[u8]) -> Result<Event, Rejection> {
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        data: Value,
    }

    let envelope: Envelope = serde_json::from_slice(body).map_err(|_| Rejection::BadJson)?;
    let payment =
        |data: Value| serde_json::from_value::<Payment>(data).map_err(|_| Rejection::BadJson);
    Ok(match envelope.kind.as_str() {
        "payment.succeeded" => Event::PaymentSucceeded(payment(envelope.data)?),
        "payment.failed" | "payment.cancelled" => Event::PaymentFailed(payment(envelope.data)?),
        kind if kind.starts_with("refund.") || kind.starts_with("dispute.") => Event::Notice {
            payment_id: envelope
                .data
                .get("payment_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            kind: envelope.kind,
        },
        _ => Event::Ignored(envelope.kind),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use axum::http::HeaderValue;

    use super::*;

    /// The Standard Webhooks reference vector.
    const VECTOR_KEY: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
    const VECTOR_ID: &str = "msg_p5jXN8AQM9LWM0D4loKWxJek";
    const VECTOR_TS: i64 = 1_614_265_330;
    const VECTOR_BODY: &[u8] = br#"{"test": 2432232314}"#;
    const VECTOR_SIG: &str = "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=";

    fn headers(id: &str, ts: &str, sig: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in [
            ("webhook-id", id),
            ("webhook-timestamp", ts),
            ("webhook-signature", sig),
        ] {
            if !value.is_empty() {
                h.insert(name, HeaderValue::from_str(value).unwrap());
            }
        }
        h
    }

    /// Headers signing `body` with `key` at `ts`, as Dodo would send them.
    pub(crate) fn signed_headers(key: &str, body: &[u8], ts: i64) -> HeaderMap {
        let key = WebhookKey::parse(key).unwrap();
        let id = format!("msg_{}", Uuid::now_v7().simple());
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key.0).unwrap();
        mac.update(format!("{id}.{ts}.").as_bytes());
        mac.update(body);
        let sig = format!("v1,{}", B64.encode(mac.finalize().into_bytes()));
        headers(&id, &ts.to_string(), &sig)
    }

    #[test]
    fn the_reference_vector_verifies() {
        let key = WebhookKey::parse(VECTOR_KEY).unwrap();
        let h = headers(VECTOR_ID, &VECTOR_TS.to_string(), VECTOR_SIG);
        assert_eq!(verify_webhook(&h, VECTOR_BODY, &key, VECTOR_TS), Ok(()));
        // A key given without its whsec_ prefix works too.
        let bare = WebhookKey::parse(VECTOR_KEY.trim_start_matches("whsec_")).unwrap();
        assert_eq!(
            verify_webhook(&h, VECTOR_BODY, &bare, VECTOR_TS + 60),
            Ok(())
        );
    }

    #[test]
    fn tampering_and_time_are_refused() {
        let key = WebhookKey::parse(VECTOR_KEY).unwrap();
        let ts = VECTOR_TS.to_string();
        let ok = headers(VECTOR_ID, &ts, VECTOR_SIG);
        assert_eq!(
            verify_webhook(&ok, br#"{"test": 2432232315}"#, &key, VECTOR_TS),
            Err(Rejection::BadSignature),
            "a changed body"
        );
        assert_eq!(
            verify_webhook(
                &headers("msg_other", &ts, VECTOR_SIG),
                VECTOR_BODY,
                &key,
                VECTOR_TS
            ),
            Err(Rejection::BadSignature),
            "a changed id"
        );
        assert_eq!(
            verify_webhook(&ok, VECTOR_BODY, &key, VECTOR_TS + 301),
            Err(Rejection::Stale),
            "too old"
        );
        assert_eq!(
            verify_webhook(&ok, VECTOR_BODY, &key, VECTOR_TS - 301),
            Err(Rejection::Stale),
            "from the future"
        );
        assert_eq!(
            verify_webhook(
                &headers(VECTOR_ID, "soon", VECTOR_SIG),
                VECTOR_BODY,
                &key,
                VECTOR_TS
            ),
            Err(Rejection::Stale)
        );
        for missing in [
            headers("", &ts, VECTOR_SIG),
            headers(VECTOR_ID, "", VECTOR_SIG),
            headers(VECTOR_ID, &ts, ""),
        ] {
            assert_eq!(
                verify_webhook(&missing, VECTOR_BODY, &key, VECTOR_TS),
                Err(Rejection::MissingHeader)
            );
        }
        let other_key = WebhookKey::parse("whsec_c2VjcmV0LWtleS1mb3ItdGVzdHM=").unwrap();
        assert_eq!(
            verify_webhook(&ok, VECTOR_BODY, &other_key, VECTOR_TS),
            Err(Rejection::BadSignature),
            "another key"
        );
    }

    #[test]
    fn any_listed_signature_can_match() {
        let key = WebhookKey::parse(VECTOR_KEY).unwrap();
        let rotated = format!("v1,bm90IHRoZSByaWdodCBvbmU= v2,ignored {VECTOR_SIG}");
        let h = headers(VECTOR_ID, &VECTOR_TS.to_string(), &rotated);
        assert_eq!(verify_webhook(&h, VECTOR_BODY, &key, VECTOR_TS), Ok(()));
    }

    #[test]
    fn keys_must_decode() {
        assert!(WebhookKey::parse("whsec_not base64!").is_err());
        assert!(WebhookKey::parse("whsec_").is_err());
        assert_eq!(
            format!("{:?}", WebhookKey::parse(VECTOR_KEY).unwrap()),
            "WebhookKey(***)"
        );
        assert_eq!(format!("{:?}", Secret::new("sk_live_x")), "***");
    }

    #[test]
    fn events_parse_tolerantly() {
        let succeeded = br#"{"business_id":"bus_1","type":"payment.succeeded","timestamp":"2026-10-07T00:00:00Z",
            "data":{"payload_type":"Payment","payment_id":"pay_1","checkout_session_id":"cks_1",
                    "total_amount":1062,"tax":62,"currency":"USD","discounts":[],
                    "product_cart":[{"product_id":"pdt_1","quantity":1}],"invoice_url":null,
                    "metadata":{"checkout_id":"x"},"brand_id":"brd_1","something_new":true}}"#;
        let Event::PaymentSucceeded(p) = parse_event(succeeded).unwrap() else {
            panic!("not a success")
        };
        assert_eq!(p.payment_id, "pay_1");
        assert_eq!(p.checkout_session_id.as_deref(), Some("cks_1"));
        assert_eq!((p.total_amount, p.tax), (Some(1062), Some(62)));
        assert_eq!(p.product_cart.unwrap()[0].product_id, "pdt_1");

        let minimal = br#"{"type":"payment.succeeded","data":{"payment_id":"pay_2"}}"#;
        let Event::PaymentSucceeded(p) = parse_event(minimal).unwrap() else {
            panic!("not a success")
        };
        assert!(p.checkout_session_id.is_none() && p.currency.is_none());

        for (body, failed) in [
            (
                &br#"{"type":"payment.failed","data":{"payment_id":"p"}}"#[..],
                true,
            ),
            (
                br#"{"type":"payment.cancelled","data":{"payment_id":"p"}}"#,
                true,
            ),
            (
                br#"{"type":"payment.processing","data":{"payment_id":"p"}}"#,
                false,
            ),
        ] {
            assert_eq!(
                matches!(parse_event(body).unwrap(), Event::PaymentFailed(_)),
                failed
            );
        }
        assert!(matches!(
            parse_event(br#"{"type":"refund.succeeded","data":{"payment_id":"pay_9"}}"#).unwrap(),
            Event::Notice { payment_id: Some(ref id), .. } if id == "pay_9"
        ));
        assert!(matches!(
            parse_event(br#"{"type":"subscription.active","data":{}}"#).unwrap(),
            Event::Ignored(ref k) if k == "subscription.active"
        ));
        assert_eq!(parse_event(b"not json").unwrap_err(), Rejection::BadJson);
        assert_eq!(
            parse_event(br#"{"type":"payment.succeeded","data":{"total_amount":5}}"#).unwrap_err(),
            Rejection::BadJson,
            "a payment needs its id"
        );
    }

    #[test]
    fn the_session_body_locks_the_price() {
        let id = Uuid::now_v7();
        let body = NewCheckout {
            product_id: "pdt_1",
            amount_cents: 237,
            email: "buyer@example.test",
            name: Some("Buyer"),
            return_url: "https://chakramcp.com/app/credits?account=a&checkout=x",
            checkout_id: id,
            account_slug: "a",
        }
        .body();
        assert_eq!(body["product_cart"][0]["amount"], 237);
        assert_eq!(body["product_cart"][0]["quantity"], 1);
        assert_eq!(body["billing_currency"], "USD");
        assert_eq!(body["feature_flags"]["allow_discount_code"], false);
        assert_eq!(body["feature_flags"]["allow_currency_selection"], false);
        assert_eq!(body["metadata"]["checkout_id"], id.to_string());
    }
}
