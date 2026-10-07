//! Buying credits (credits P4): the checkout routes for the web app and
//! Dodo's webhook. The work lives in [`crate::purchases`]. Spec:
//! `docs/superpowers/specs/2026-10-06-credits-p4-purchasing-design.md` §5, §6.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};
use chakramcp_shared::telemetry;

use crate::auth::{AuthUser, InteractiveUser};
use crate::purchases::dodo::{self, Event};
use crate::purchases::{self, Buyer, CheckoutStatus, CreatedCheckout, Outcome};
use crate::state::AppState;

/// The account `slug` names, if `user_id` is a member.
async fn member_account(state: &AppState, slug: &str, user_id: Uuid) -> ApiResult<Uuid> {
    sqlx::query_scalar!(
        r#"
        SELECT a.id FROM accounts a
          JOIN account_memberships m ON m.account_id = a.id
         WHERE a.slug = $1 AND m.user_id = $2
        "#,
        slug,
        user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)
}

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub amount_cents: i64,
}

// ─────────────────────────────────────────────────────────
// POST /v1/orgs/{slug}/credits/checkouts — any member, signed in to the web app
// ─────────────────────────────────────────────────────────
pub async fn create_checkout(
    State(state): State<AppState>,
    InteractiveUser(user): InteractiveUser,
    Path(slug): Path<String>,
    Json(req): Json<CheckoutRequest>,
) -> ApiResult<(StatusCode, Json<CreatedCheckout>)> {
    let cfg = state.purchase.clone().ok_or(ApiError::NotFound)?;
    let account_id = member_account(&state, &slug, user.user_id).await?;
    let name = sqlx::query_scalar!("SELECT display_name FROM users WHERE id = $1", user.user_id)
        .fetch_optional(&state.db)
        .await?
        .filter(|n| !n.trim().is_empty());
    let created = purchases::create_checkout(
        &state.db,
        &cfg,
        &state.config.frontend_base_url,
        account_id,
        &slug,
        Buyer {
            user_id: user.user_id,
            email: &user.email,
            name: name.as_deref(),
        },
        req.amount_cents,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

// ─────────────────────────────────────────────────────────
// GET /v1/orgs/{slug}/credits/checkouts/{id} — any member
// ─────────────────────────────────────────────────────────
pub async fn checkout_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path((slug, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<CheckoutStatus>> {
    if state.purchase.is_none() {
        return Err(ApiError::NotFound);
    }
    let account_id = member_account(&state, &slug, user.user_id).await?;
    Ok(Json(
        purchases::checkout_status(&state.db, account_id, id).await?,
    ))
}

// ─────────────────────────────────────────────────────────
// POST /v1/webhooks/dodo — Dodo's signature is the only way in
// ─────────────────────────────────────────────────────────
pub async fn dodo_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(cfg) = state.purchase.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let webhook_id = headers
        .get("webhook-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if let Err(rejection) = dodo::verify_webhook(
        &headers,
        &body,
        &cfg.webhook_key,
        chrono::Utc::now().timestamp(),
    ) {
        telemetry::record_webhook_rejected(rejection.reason());
        tracing::warn!(
            event = "credits.webhook_rejected",
            reason = rejection.reason(),
            webhook_id,
            "Dodo webhook refused"
        );
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let event = match dodo::parse_event(&body) {
        Ok(event) => event,
        Err(rejection) => {
            // Signed by Dodo, so ours to fix: a 500 keeps Dodo retrying
            // (about 27 hours) while a fix ships.
            telemetry::record_webhook_rejected(rejection.reason());
            tracing::error!(
                event = "credits.webhook_unparsed",
                webhook_id,
                "a signed Dodo webhook we couldn't parse"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    match event {
        Event::PaymentSucceeded(payment) => {
            let outcome = match purchases::apply_succeeded(&state.db, &cfg, &payment).await {
                Ok(outcome) => outcome,
                Err(e) => return e.into_response(),
            };
            let payment_id = payment.payment_id.as_str();
            match outcome {
                Outcome::Paid {
                    account_id,
                    checkout_id,
                    credits_mc,
                } => {
                    telemetry::record_purchase("paid");
                    tracing::info!(
                        event = "credits.purchased",
                        %account_id,
                        %checkout_id,
                        payment_id,
                        credits_mc,
                        "credits purchased"
                    );
                }
                Outcome::Duplicate => {
                    tracing::info!(payment_id, webhook_id, "Dodo redelivered a handled payment");
                }
                Outcome::OtherProduct => {
                    tracing::info!(payment_id, "a Dodo payment for another product: ignored");
                }
                Outcome::Unmatched(why) => {
                    telemetry::record_purchase("unmatched");
                    tracing::error!(
                        event = "credits.payment_unmatched",
                        payment_id,
                        checkout_session_id = payment.checkout_session_id.as_deref(),
                        why,
                        "a credits payment matching none of our open checkouts: not credited, \
                         refund it in Dodo or grant the credits by hand"
                    );
                }
                Outcome::Unapplied {
                    checkout_id,
                    reason,
                } => {
                    telemetry::record_purchase("unapplied");
                    tracing::error!(
                        event = "credits.payment_unapplied",
                        %checkout_id,
                        payment_id,
                        reason,
                        "a paid checkout we didn't credit: refund it in Dodo or grant the \
                         credits by hand"
                    );
                }
            }
            StatusCode::OK.into_response()
        }
        Event::PaymentFailed(payment) => match purchases::apply_failed(&state.db, &payment).await {
            Ok(changed) => {
                if changed {
                    telemetry::record_purchase("failed");
                }
                StatusCode::OK.into_response()
            }
            Err(e) => e.into_response(),
        },
        Event::Notice { kind, payment_id } => {
            tracing::warn!(
                event = "credits.payment_notice",
                kind,
                payment_id = payment_id.as_deref(),
                "a refund or dispute: adjust the account's credits by hand if needed"
            );
            StatusCode::OK.into_response()
        }
        Event::Ignored(kind) => {
            tracing::debug!(kind, "Dodo event ignored");
            StatusCode::OK.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{header, Method, Request};
    use axum::routing::post;
    use axum::Router;
    use chakramcp_shared::credits::CreditsConfig;
    use chakramcp_shared::hosting::{HostingMode, HostingSettings};
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::purchases::config::{Mode, PurchaseConfig};
    use crate::purchases::dodo::{self, DodoClient, Secret, WebhookKey};
    use crate::tests_support::*;
    use crate::AppState;

    const KEY: &str = "whsec_c2VjcmV0LWtleS1mb3ItdGVzdHM=";
    const PRODUCT: &str = "pdt_credits";

    /// A stand-in for Dodo's API: records each checkout request and answers
    /// with a new session, or with a 500 when `failing`.
    #[derive(Clone)]
    struct FakeDodo {
        base_url: String,
        requests: Arc<Mutex<Vec<Value>>>,
    }

    async fn fake_dodo(failing: bool) -> FakeDodo {
        let requests: Arc<Mutex<Vec<Value>>> = Arc::default();
        let seen = requests.clone();
        let app = Router::new().route(
            "/checkouts",
            post(move |axum::Json(body): axum::Json<Value>| {
                let seen = seen.clone();
                async move {
                    let n = {
                        let mut seen = seen.lock().unwrap();
                        seen.push(body);
                        seen.len()
                    };
                    if failing {
                        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({})));
                    }
                    let session = format!("cks_test_{n}_{}", Uuid::now_v7().simple());
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(json!({
                            "session_id": session,
                            "checkout_url": format!("https://test.checkout.dodopayments.com/session/{session}"),
                        })),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeDodo {
            base_url: format!("http://{addr}"),
            requests,
        }
    }

    fn purchase_config(base_url: &str) -> PurchaseConfig {
        PurchaseConfig {
            client: DodoClient::new(base_url, Secret::new("test-key")).unwrap(),
            webhook_key: WebhookKey::parse(KEY).unwrap(),
            product_id: PRODUCT.into(),
            mode: Mode::Test,
            credits_per_usd: 1_000,
            min_cents: 100,
            max_cents: 500_000,
        }
    }

    /// chakramcp.com: managed, credits on, Dodo configured.
    fn selling(pool: &PgPool, dodo: &FakeDodo) -> AppState {
        AppState::new(pool.clone(), test_config())
            .with_hosting(HostingSettings::for_mode(HostingMode::Managed))
            .with_credits_config(CreditsConfig::default())
            .with_purchase(Some(purchase_config(&dodo.base_url)))
    }

    async fn call(
        state: &AppState,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (axum::http::StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        let body = match body {
            Some(json) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let res = crate::router(state.clone())
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    async fn slug_of(pool: &PgPool, account: Uuid) -> String {
        sqlx::query_scalar("SELECT slug FROM accounts WHERE id = $1")
            .bind(account)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Buy `cents` for `account`; returns the checkout id and Dodo session id.
    async fn checkout(state: &AppState, token: &str, account: Uuid, cents: i64) -> (Uuid, String) {
        let slug = slug_of(&state.db, account).await;
        let (status, created) = call(
            state,
            Method::POST,
            &format!("/v1/orgs/{slug}/credits/checkouts"),
            token,
            Some(json!({ "amount_cents": cents })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{created}");
        let id: Uuid = created["checkout_id"].as_str().unwrap().parse().unwrap();
        let session: String =
            sqlx::query_scalar("SELECT dodo_session_id FROM credit_checkouts WHERE id = $1")
                .bind(id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        (id, session)
    }

    /// A Dodo payment event for `session`, paid in full in USD.
    fn payment(kind: &str, payment_id: &str, session: Option<&str>, total: i64) -> Value {
        json!({
            "business_id": "bus_test",
            "type": kind,
            "timestamp": "2026-10-07T00:00:00Z",
            "data": {
                "payload_type": "Payment",
                "payment_id": payment_id,
                "checkout_session_id": session,
                "total_amount": total,
                "tax": 0,
                "currency": "USD",
                "discounts": [],
                "product_cart": [{ "product_id": PRODUCT, "quantity": 1 }],
                "invoice_url": format!("https://test.dodopayments.com/invoices/payments/{payment_id}"),
                "metadata": {},
            }
        })
    }

    async fn deliver(state: &AppState, event: &Value) -> axum::http::StatusCode {
        let body = event.to_string().into_bytes();
        let headers = dodo::tests::signed_headers(KEY, &body, chrono::Utc::now().timestamp());
        let mut req = Request::builder()
            .method(Method::POST)
            .uri("/v1/webhooks/dodo");
        for (name, value) in &headers {
            req = req.header(name, value);
        }
        crate::router(state.clone())
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap()
            .status()
    }

    async fn purchase_rows(pool: &PgPool, account: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM credit_ledger WHERE account_id = $1 AND reason = 'purchase'",
        )
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn balance(pool: &PgPool, account: Uuid) -> Option<i64> {
        sqlx::query_scalar("SELECT balance_mc FROM credit_wallets WHERE account_id = $1")
            .bind(account)
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    async fn checkout_row(pool: &PgPool, id: Uuid) -> (String, Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT status, unapplied_reason, dodo_payment_id FROM credit_checkouts WHERE id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_checkout_is_recorded_then_opened_at_dodo(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (user, token, account) = seed_user_with_personal(&pool, "buyer").await;
        let slug = slug_of(&pool, account).await;

        let (status, created) = call(
            &state,
            Method::POST,
            &format!("/v1/orgs/{slug}/credits/checkouts"),
            &token,
            Some(json!({ "amount_cents": 237 })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::CREATED, "{created}");
        assert_eq!(created["credits_mc"], 2_370_000, "$2.37 buys 2,370 credits");
        let id = created["checkout_id"].as_str().unwrap();
        assert!(created["checkout_url"]
            .as_str()
            .unwrap()
            .starts_with("https://test.checkout"));

        let sent = dodo.requests.lock().unwrap()[0].clone();
        assert_eq!(sent["product_cart"][0]["product_id"], PRODUCT);
        assert_eq!(sent["product_cart"][0]["amount"], 237);
        assert_eq!(sent["billing_currency"], "USD");
        assert_eq!(sent["feature_flags"]["allow_discount_code"], false);
        assert_eq!(sent["feature_flags"]["allow_currency_selection"], false);
        assert_eq!(sent["metadata"]["checkout_id"], id);
        let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sent["customer"]["email"], email.as_str());
        assert_eq!(
            sent["return_url"],
            format!("http://localhost:3000/app/credits?account={slug}&checkout={id}")
        );

        let (status, view) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits"),
            &token,
            None,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(view["purchase"]["mode"], "test");
        assert_eq!(view["purchase"]["min_cents"], 100);
        assert_eq!(view["payments"][0]["id"], id);
        assert_eq!(view["payments"][0]["status"], "open");
        assert_eq!(view["payments"][0]["buyer_email"], email.as_str());

        let (status, polled) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits/checkouts/{id}"),
            &token,
            None,
        )
        .await;
        assert_eq!(
            (status, polled["status"].clone()),
            (axum::http::StatusCode::OK, json!("open"))
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn limits_and_the_hourly_throttle_hold(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "buyer").await;
        let path = format!(
            "/v1/orgs/{}/credits/checkouts",
            slug_of(&pool, account).await
        );
        for cents in [99_i64, 500_001, -5, 4_294_967_396] {
            let (status, body) = call(
                &state,
                Method::POST,
                &path,
                &token,
                Some(json!({ "amount_cents": cents })),
            )
            .await;
            assert_eq!(
                status,
                axum::http::StatusCode::BAD_REQUEST,
                "{cents}: {body}"
            );
        }
        for _ in 0..crate::purchases::CHECKOUTS_PER_HOUR {
            checkout(&state, &token, account, 100).await;
        }
        let (status, body) = call(
            &state,
            Method::POST,
            &path,
            &token,
            Some(json!({ "amount_cents": 100 })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["error"]["code"], "too_many_checkouts");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_dodo_failure_marks_the_checkout_failed(pool: PgPool) {
        let dodo = fake_dodo(true).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "buyer").await;
        let path = format!(
            "/v1/orgs/{}/credits/checkouts",
            slug_of(&pool, account).await
        );
        let (status, body) = call(
            &state,
            Method::POST,
            &path,
            &token,
            Some(json!({ "amount_cents": 500 })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"]["code"], "payment_provider_error");
        let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM credit_checkouts")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(statuses, vec!["failed".to_owned()]);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn only_people_signed_in_to_the_web_app_can_buy(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (user, session, account) = seed_user_with_personal(&pool, "owner").await;
        let path = format!(
            "/v1/orgs/{}/credits/checkouts",
            slug_of(&pool, account).await
        );
        let body = Some(json!({ "amount_cents": 1_000 }));

        let (_, api_key) = seed_api_key(&pool, user.user_id, "ci").await;
        let agent = seed_agent(&pool, account, "helper", user.user_id).await;
        let pairing = seed_approved_device_flow(&pool, user.user_id, agent).await;
        let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let paired = jwt::UserClaims::new(user.user_id, email, false, 1);
        sqlx::query("UPDATE oauth_device_codes SET minted_jti = $1 WHERE id = $2")
            .bind(paired.jti)
            .bind(pairing)
            .execute(&pool)
            .await
            .unwrap();
        let paired = jwt::encode_jwt(&paired, TEST_SECRET).unwrap();
        for (who, token) in [
            ("an API key", &api_key),
            ("a paired device or CLI", &paired),
        ] {
            let (status, _) = call(&state, Method::POST, &path, token, body.clone()).await;
            assert_eq!(
                status,
                axum::http::StatusCode::FORBIDDEN,
                "{who} started a checkout"
            );
        }
        let (_, stranger, _) = seed_user_with_personal(&pool, "stranger").await;
        let (status, _) = call(&state, Method::POST, &path, &stranger, body.clone()).await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND, "a non-member");
        assert!(dodo.requests.lock().unwrap().is_empty());

        // Any member of an org can buy for it, from the web app.
        let org = seed_org(
            &pool,
            &format!("team-{}", &Uuid::now_v7().simple().to_string()[..8]),
            user.user_id,
        )
        .await;
        let (_, member, _) = seed_user_with_personal(&pool, "member").await;
        let member_id: Uuid = jwt::decode_jwt(&member, TEST_SECRET).unwrap().sub;
        sqlx::query("INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'member')")
            .bind(Uuid::now_v7())
            .bind(org)
            .bind(member_id)
            .execute(&pool)
            .await
            .unwrap();
        checkout(&state, &member, org, 1_000).await;
        checkout(&state, &session, account, 1_000).await;
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn without_purchasing_every_route_is_not_found(pool: PgPool) {
        let state = AppState::new(pool.clone(), test_config()); // self-hosted defaults
        let (_, token, account) = seed_user_with_personal(&pool, "selfhost").await;
        let slug = slug_of(&pool, account).await;
        let (status, _) = call(
            &state,
            Method::POST,
            &format!("/v1/orgs/{slug}/credits/checkouts"),
            &token,
            Some(json!({ "amount_cents": 1_000 })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        let (status, _) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits/checkouts/{}", Uuid::now_v7()),
            &token,
            None,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(
            deliver(
                &state,
                &payment("payment.succeeded", "pay_x", Some("cks_x"), 100)
            )
            .await,
            axum::http::StatusCode::NOT_FOUND
        );
        let (_, view) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits"),
            &token,
            None,
        )
        .await;
        assert_eq!(view["purchase"], Value::Null);
        assert_eq!(view["payments"], json!([]));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_payment_is_credited_exactly_once(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "buyer").await;
        let (id, session) = checkout(&state, &token, account, 237).await;
        let paid = payment("payment.succeeded", "pay_1", Some(&session), 237);

        for _ in 0..2 {
            assert_eq!(deliver(&state, &paid).await, axum::http::StatusCode::OK);
        }
        assert_eq!(purchase_rows(&pool, account).await, 1);
        assert_eq!(balance(&pool, account).await, Some(2_370_000));
        let (status, _, payment_id) = checkout_row(&pool, id).await;
        assert_eq!(
            (status.as_str(), payment_id.as_deref()),
            ("paid", Some("pay_1"))
        );

        let slug = slug_of(&pool, account).await;
        let (_, view) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits"),
            &token,
            None,
        )
        .await;
        assert_eq!(
            view["balance_mc"],
            2_370_000 + 5_000_000,
            "the first monthly grant, seconds away, counts: buying never shows less than before"
        );
        assert_eq!(view["payments"][0]["status"], "paid");
        assert!(view["payments"][0]["invoice_url"]
            .as_str()
            .unwrap()
            .ends_with("pay_1"));
        let entry = &view["ledger"][0];
        assert_eq!(entry["kind"], "purchase");
        assert_eq!(entry["delta_mc"], 2_370_000);
        assert_eq!(entry["purchase"]["amount_cents"], 237);
        assert_eq!(entry["purchase"]["currency"], "USD");
        assert!(entry["purchase"]["buyer_email"]
            .as_str()
            .unwrap()
            .contains('@'));
        assert!(
            entry.get("by").is_none(),
            "who made admin changes stays admin-only"
        );

        // balance = Σ ledger − Σ charges
        let reconciled: i64 = sqlx::query_scalar(
            "SELECT (COALESCE((SELECT SUM(delta_mc) FROM credit_ledger WHERE account_id = $1), 0)
                   - COALESCE((SELECT SUM(cost_mc) FROM invocation_charges WHERE account_id = $1), 0))::bigint",
        )
        .bind(account)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(Some(reconciled), balance(&pool, account).await);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn concurrent_deliveries_credit_once(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let wide = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap();
        let state = selling(&wide, &dodo);
        for run in 0..5 {
            let (_, token, account) = seed_user_with_personal(&pool, &format!("race{run}")).await;
            let (_, session) = checkout(&state, &token, account, 500).await;
            let paid = payment(
                "payment.succeeded",
                &format!("pay_race_{run}"),
                Some(&session),
                500,
            );
            let (a, b) = tokio::join!(deliver(&state, &paid), deliver(&state, &paid));
            assert_eq!(
                (a, b),
                (axum::http::StatusCode::OK, axum::http::StatusCode::OK)
            );
            assert_eq!(purchase_rows(&pool, account).await, 1, "run {run}");
            assert_eq!(balance(&pool, account).await, Some(5_000_000), "run {run}");
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_failed_attempt_can_still_be_paid(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "retry").await;
        let (id, session) = checkout(&state, &token, account, 1_000).await;

        let declined = payment("payment.failed", "pay_declined", Some(&session), 1_000);
        assert_eq!(deliver(&state, &declined).await, axum::http::StatusCode::OK);
        assert_eq!(checkout_row(&pool, id).await.0, "failed");

        let paid = payment("payment.succeeded", "pay_ok", Some(&session), 1_000);
        assert_eq!(deliver(&state, &paid).await, axum::http::StatusCode::OK);
        assert_eq!(checkout_row(&pool, id).await.0, "paid");
        assert_eq!(purchase_rows(&pool, account).await, 1);

        // A late failure for the same session changes nothing.
        assert_eq!(deliver(&state, &declined).await, axum::http::StatusCode::OK);
        assert_eq!(checkout_row(&pool, id).await.0, "paid");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_second_payment_on_a_settled_checkout_is_not_credited(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "twice").await;
        let (id, session) = checkout(&state, &token, account, 1_000).await;
        for payment_id in ["pay_first", "pay_second"] {
            let paid = payment("payment.succeeded", payment_id, Some(&session), 1_000);
            assert_eq!(deliver(&state, &paid).await, axum::http::StatusCode::OK);
        }
        assert_eq!(purchase_rows(&pool, account).await, 1);
        assert_eq!(
            checkout_row(&pool, id).await.2.as_deref(),
            Some("pay_first")
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn payments_not_as_agreed_are_held_for_review(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "odd").await;
        type Tweak = (&'static str, fn(&mut Value));
        let tweaks: [Tweak; 4] = [
            ("a discount", |d| {
                d["discounts"] = json!([{ "discount_id": "dsc_1", "amount": 900 }])
            }),
            ("another currency", |d| d["currency"] = json!("EUR")),
            ("less than the amount", |d| d["total_amount"] = json!(999)),
            ("no currency", |d| d["currency"] = Value::Null),
        ];
        for (n, (what, tweak)) in tweaks.into_iter().enumerate() {
            let (id, session) = checkout(&state, &token, account, 1_000).await;
            let mut event = payment(
                "payment.succeeded",
                &format!("pay_odd_{n}"),
                Some(&session),
                1_000,
            );
            tweak(&mut event["data"]);
            assert_eq!(
                deliver(&state, &event).await,
                axum::http::StatusCode::OK,
                "{what}"
            );
            let (status, reason, payment_id) = checkout_row(&pool, id).await;
            assert_eq!(
                (status.as_str(), reason.as_deref()),
                ("unapplied", Some("not_as_agreed")),
                "{what}"
            );
            assert!(payment_id.is_some());
            // A redelivery is recognised, and a second payment isn't credited either.
            assert_eq!(deliver(&state, &event).await, axum::http::StatusCode::OK);
            let other = payment(
                "payment.succeeded",
                &format!("pay_odd_{n}_b"),
                Some(&session),
                1_000,
            );
            assert_eq!(deliver(&state, &other).await, axum::http::StatusCode::OK);
        }
        assert_eq!(purchase_rows(&pool, account).await, 0);
        assert_eq!(balance(&pool, account).await, None);
        // A total above the amount (tax added on top) is fine.
        let (id, session) = checkout(&state, &token, account, 1_000).await;
        let taxed = payment("payment.succeeded", "pay_taxed", Some(&session), 1_180);
        assert_eq!(deliver(&state, &taxed).await, axum::http::StatusCode::OK);
        assert_eq!(checkout_row(&pool, id).await.0, "paid");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn payments_we_cant_place_are_never_credited(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, token, account) = seed_user_with_personal(&pool, "nobody").await;
        checkout(&state, &token, account, 1_000).await;

        let unknown_session = payment("payment.succeeded", "pay_u1", Some("cks_not_ours"), 1_000);
        let mut no_session = payment("payment.succeeded", "pay_u2", None, 1_000);
        no_session["data"]["product_cart"] = Value::Null;
        let mut empty_cart = payment("payment.succeeded", "pay_u3", None, 1_000);
        empty_cart["data"]["product_cart"] = json!([]);
        let mut other_product = payment("payment.succeeded", "pay_u4", None, 1_000);
        other_product["data"]["product_cart"] =
            json!([{ "product_id": "pdt_something_else", "quantity": 1 }]);
        for event in [unknown_session, no_session, empty_cart, other_product] {
            assert_eq!(deliver(&state, &event).await, axum::http::StatusCode::OK);
        }
        let ledger: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ledger, 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_payment_for_a_deleted_account_is_held(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (user, token, _) = seed_user_with_personal(&pool, "founder").await;
        let org = seed_org(
            &pool,
            &format!("gone-{}", &Uuid::now_v7().simple().to_string()[..8]),
            user.user_id,
        )
        .await;
        let (id, session) = checkout(&state, &token, org, 2_000).await;
        sqlx::query("DELETE FROM accounts WHERE id = $1")
            .bind(org)
            .execute(&pool)
            .await
            .unwrap();
        let paid = payment("payment.succeeded", "pay_gone", Some(&session), 2_000);
        assert_eq!(deliver(&state, &paid).await, axum::http::StatusCode::OK);
        let (status, reason, _) = checkout_row(&pool, id).await;
        assert_eq!(
            (status.as_str(), reason.as_deref()),
            ("unapplied", Some("account_gone"))
        );
        assert_eq!(balance(&pool, org).await, None);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn webhooks_need_dodos_signature(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let event = payment("payment.succeeded", "pay_sig", Some("cks_sig"), 100).to_string();
        let send = |headers: Vec<(&'static str, String)>, body: String| {
            let state = state.clone();
            async move {
                let mut req = Request::builder()
                    .method(Method::POST)
                    .uri("/v1/webhooks/dodo");
                for (name, value) in headers {
                    req = req.header(name, value);
                }
                crate::router(state)
                    .oneshot(req.body(Body::from(body)).unwrap())
                    .await
                    .unwrap()
                    .status()
            }
        };
        assert_eq!(
            send(vec![], event.clone()).await,
            axum::http::StatusCode::UNAUTHORIZED
        );

        let now = chrono::Utc::now().timestamp();
        let as_headers = |h: axum::http::HeaderMap| {
            h.iter()
                .map(|(k, v)| {
                    (
                        match k.as_str() {
                            "webhook-id" => "webhook-id",
                            "webhook-timestamp" => "webhook-timestamp",
                            _ => "webhook-signature",
                        },
                        v.to_str().unwrap().to_owned(),
                    )
                })
                .collect::<Vec<_>>()
        };
        // Built at run time: secret-shaped literals trip the secret scanner.
        let other_key = format!(
            "whsec_{}",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "another key")
        );
        let wrong = dodo::tests::signed_headers(&other_key, event.as_bytes(), now);
        assert_eq!(
            send(as_headers(wrong), event.clone()).await,
            axum::http::StatusCode::UNAUTHORIZED
        );
        let stale = dodo::tests::signed_headers(KEY, event.as_bytes(), now - 3_600);
        assert_eq!(
            send(as_headers(stale), event.clone()).await,
            axum::http::StatusCode::UNAUTHORIZED
        );

        let garbage = "{not json".to_owned();
        let signed = dodo::tests::signed_headers(KEY, garbage.as_bytes(), now);
        assert_eq!(
            send(as_headers(signed), garbage).await,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "signed by Dodo but unreadable: retry"
        );
        let other = json!({ "type": "subscription.active", "data": {} }).to_string();
        let signed = dodo::tests::signed_headers(KEY, other.as_bytes(), now);
        assert_eq!(
            send(as_headers(signed), other).await,
            axum::http::StatusCode::OK
        );
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_checkout_belongs_to_its_account(pool: PgPool) {
        let dodo = fake_dodo(false).await;
        let state = selling(&pool, &dodo);
        let (_, mine, account) = seed_user_with_personal(&pool, "mine").await;
        let (_, theirs, their_account) = seed_user_with_personal(&pool, "theirs").await;
        let (id, _) = checkout(&state, &mine, account, 1_000).await;
        let their_slug = slug_of(&pool, their_account).await;
        let (status, _) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{their_slug}/credits/checkouts/{id}"),
            &theirs,
            None,
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);

        // An open checkout over a day old reads as expired.
        sqlx::query(
            "UPDATE credit_checkouts SET created_at = now() - interval '25 hours' WHERE id = $1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
        let slug = slug_of(&pool, account).await;
        let (_, polled) = call(
            &state,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits/checkouts/{id}"),
            &mine,
            None,
        )
        .await;
        assert_eq!(polled["status"], "expired");
    }
}
