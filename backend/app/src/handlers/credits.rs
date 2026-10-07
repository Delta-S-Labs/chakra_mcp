//! Credits for people: what an account has and spends (owners, P2) and how
//! the operator manages it (admins, P3). The reads and writes live in
//! [`crate::credits_service`], which `chakramcp-server credits` shares.

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{AdminUser, AuthUser};
use crate::credits_service::{self, Actor, CreditsView, SettingsChange};
use crate::state::AppState;

// ─────────────────────────────────────────────────────────
// GET /v1/orgs/{slug}/credits — any member of the account
// ─────────────────────────────────────────────────────────
pub async fn account_credits(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> ApiResult<Json<CreditsView>> {
    let account_id = sqlx::query_scalar!(
        r#"
        SELECT a.id FROM accounts a
          JOIN account_memberships m ON m.account_id = a.id
         WHERE a.slug = $1 AND m.user_id = $2
        "#,
        slug,
        user.user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let mut view = credits_service::load_view(&state.db, &state.credits, account_id, false).await?;
    view.purchase = state.purchase.as_ref().map(|p| p.info());
    Ok(Json(view))
}

// ─────────────────────────────────────────────────────────
// GET /v1/admin/accounts/{account_id}/credits
// ─────────────────────────────────────────────────────────
pub async fn admin_account_credits(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(account_id): Path<Uuid>,
) -> ApiResult<Json<CreditsView>> {
    credits_service::ensure_account(&state.db, account_id).await?;
    Ok(Json(
        credits_service::load_view(&state.db, &state.credits, account_id, true).await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct LedgerEntryRequest {
    /// `grant` adds credits; `adjustment` moves the balance either way
    /// (e.g. correcting after a refund) and needs a note.
    pub kind: String,
    pub amount_mc: i64,
    /// Shown to the account's members with the entry.
    pub note: Option<String>,
}

// ─────────────────────────────────────────────────────────
// POST /v1/admin/accounts/{account_id}/credits/ledger
// ─────────────────────────────────────────────────────────
pub async fn admin_add_entry(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(account_id): Path<Uuid>,
    Json(req): Json<LedgerEntryRequest>,
) -> ApiResult<Json<CreditsView>> {
    let actor = Actor::Admin {
        user_id: admin.user_id,
        email: admin.email,
    };
    credits_service::add_entry(
        &state.db,
        account_id,
        &req.kind,
        req.amount_mc,
        req.note,
        &actor,
    )
    .await?;
    Ok(Json(
        credits_service::load_view(&state.db, &state.credits, account_id, true).await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct SettingsRequest {
    /// Absent = unchanged; `null` = back to the default.
    #[serde(default, deserialize_with = "present")]
    pub monthly_free_grant_mc: Option<Option<i64>>,
    /// Absent = unchanged; `null` = back to the default.
    #[serde(default, deserialize_with = "present")]
    pub rate_limit_per_min: Option<Option<i32>>,
    pub unlimited: Option<bool>,
    pub note: Option<String>,
}

/// Tells an explicit `null` (`Some(None)`) apart from an absent field
/// (`None`, via `#[serde(default)]`).
fn present<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

// ─────────────────────────────────────────────────────────
// PATCH /v1/admin/accounts/{account_id}/credits
// ─────────────────────────────────────────────────────────
pub async fn admin_update_settings(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(account_id): Path<Uuid>,
    Json(req): Json<SettingsRequest>,
) -> ApiResult<Json<CreditsView>> {
    let actor = Actor::Admin {
        user_id: admin.user_id,
        email: admin.email,
    };
    let change = SettingsChange {
        monthly_free_grant_mc: req.monthly_free_grant_mc,
        rate_limit_per_min: req.rate_limit_per_min,
        unlimited: req.unlimited,
        note: req.note,
    };
    credits_service::update_settings(&state.db, account_id, change, &actor).await?;
    Ok(Json(
        credits_service::load_view(&state.db, &state.credits, account_id, true).await?,
    ))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Method, Request, StatusCode};
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::tests_support::*;

    async fn call(
        pool: &PgPool,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let app = crate::router(crate::AppState::new(pool.clone(), test_config()));
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
        let res = app.oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    async fn slug_of(pool: &PgPool, account: Uuid) -> String {
        sqlx::query_scalar("SELECT slug FROM accounts WHERE id = $1")
            .bind(account)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn member_view(pool: &PgPool, token: &str, account: Uuid) -> Value {
        let slug = slug_of(pool, account).await;
        let (status, view) = call(
            pool,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits"),
            token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        view
    }

    /// A user whose JWT carries the admin claim, as `ADMIN_EMAIL` grants at
    /// sign-in. Returns (user id, token, email).
    async fn admin(pool: &PgPool) -> (Uuid, String, String) {
        let (user, _, _) = seed_user_with_personal(pool, "admin").await;
        let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
            .bind(user.user_id)
            .fetch_one(pool)
            .await
            .unwrap();
        let claims = jwt::UserClaims::new(user.user_id, email.clone(), true, 1);
        let token = jwt::encode_jwt(&claims, TEST_SECRET).unwrap();
        (user.user_id, token, email)
    }

    async fn wallet(pool: &PgPool, account: Uuid, balance_mc: i64, granted: bool) {
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period)
             VALUES ($1, $2, CASE WHEN $3 THEN date_trunc('month', now() AT TIME ZONE 'UTC')::date END)",
        )
        .bind(account)
        .bind(balance_mc)
        .bind(granted)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn charge(pool: &PgPool, account: Uuid, n: usize) {
        for _ in 0..n {
            sqlx::query(
                "INSERT INTO invocation_charges (invocation_id, account_id, cost_mc) VALUES ($1, $2, 100)",
            )
            .bind(Uuid::now_v7())
            .bind(account)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn a_new_account_shows_the_grant_its_first_invocation_brings(pool: PgPool) {
        let (_, token, account) = seed_user_with_personal(&pool, "fresh").await;
        let view = member_view(&pool, &token, account).await;
        assert_eq!(view["has_wallet"], false);
        assert_eq!(view["balance_mc"], 5_000_000);
        assert_eq!(view["status"], "active");
        assert_eq!(view["monthly_free_grant_mc"], 5_000_000);
        assert_eq!(view["monthly_free_grant_override_mc"], Value::Null);
        assert_eq!(view["rate_limit_per_min"], 60);
        assert_eq!(view["cost_per_invocation_mc"], 100);
        assert_eq!(view["spent_this_month_mc"], 0);
        assert_eq!(view["invocations_this_month"], 0);
        assert!(view["next_grant_on"].is_string());
        assert_eq!(view["daily"], json!([]));
        assert_eq!(view["ledger"], json!([]));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn members_see_balance_spend_and_history(pool: PgPool) {
        let (_, token, account) = seed_user_with_personal(&pool, "spender").await;
        wallet(&pool, account, 100_000 - 300, true).await;
        sqlx::query(
            "INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc, metadata)
             VALUES ($1, 100000, 'free_grant', 100000,
                     jsonb_build_object('period', date_trunc('month', now() AT TIME ZONE 'UTC')::date))",
        )
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
        charge(&pool, account, 3).await;

        let view = member_view(&pool, &token, account).await;
        assert_eq!(view["has_wallet"], true);
        assert_eq!(view["balance_mc"], 99_700);
        assert_eq!(view["spent_this_month_mc"], 300);
        assert_eq!(view["invocations_this_month"], 3);
        assert_eq!(view["daily"].as_array().unwrap().len(), 1);
        assert_eq!(view["daily"][0]["invocations"], 3);
        assert_eq!(view["daily"][0]["spent_mc"], 300);
        let entry = &view["ledger"][0];
        assert_eq!(entry["kind"], "free_grant");
        assert_eq!(entry["delta_mc"], 100_000);
        assert!(entry["period"].is_string());
        assert!(entry.get("by").is_none());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn the_status_matches_the_relay_switch(pool: PgPool) {
        let (_, token, account) = seed_user_with_personal(&pool, "broke").await;
        wallet(&pool, account, 50, true).await;
        let view = member_view(&pool, &token, account).await;
        assert_eq!(
            view["status"], "blocked",
            "50 mc can't pay for a 100 mc call"
        );

        sqlx::query("UPDATE credit_wallets SET unlimited = true WHERE account_id = $1")
            .bind(account)
            .execute(&pool)
            .await
            .unwrap();
        let view = member_view(&pool, &token, account).await;
        assert_eq!(view["status"], "unlimited");

        // A wallet the worker just created isn't blocked before its first grant.
        let (_, token, fresh) = seed_user_with_personal(&pool, "fresh").await;
        wallet(&pool, fresh, -100, false).await;
        let view = member_view(&pool, &token, fresh).await;
        assert_eq!(view["status"], "active");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn other_peoples_accounts_are_not_found(pool: PgPool) {
        let (_, _, theirs) = seed_user_with_personal(&pool, "owner").await;
        let (_, stranger, _) = seed_user_with_personal(&pool, "stranger").await;
        let slug = slug_of(&pool, theirs).await;
        let (status, _) = call(
            &pool,
            Method::GET,
            &format!("/v1/orgs/{slug}/credits"),
            &stranger,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn admin_endpoints_refuse_everyone_else(pool: PgPool) {
        let (_, token, account) = seed_user_with_personal(&pool, "owner").await;
        let base = format!("/v1/admin/accounts/{account}/credits");
        let requests = [
            (Method::GET, base.clone(), None),
            (
                Method::PATCH,
                base.clone(),
                Some(json!({ "unlimited": true })),
            ),
            (
                Method::POST,
                format!("{base}/ledger"),
                Some(json!({ "kind": "grant", "amount_mc": 1000 })),
            ),
        ];
        for (method, path, body) in requests {
            let (status, _) = call(&pool, method.clone(), &path, &token, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        let wallets: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_wallets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(wallets, 0, "an owner can't top up their own account");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn an_admin_grant_adds_credits_and_records_who(pool: PgPool) {
        let (_, member, account) = seed_user_with_personal(&pool, "customer").await;
        let (admin_id, admin_token, admin_email) = admin(&pool).await;
        let (status, view) = call(
            &pool,
            Method::POST,
            &format!("/v1/admin/accounts/{account}/credits/ledger"),
            &admin_token,
            Some(json!({ "kind": "grant", "amount_mc": 5_000, "note": " welcome bonus " })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["has_wallet"], true);
        // The new wallet's monthly grant lands on the worker's next pass;
        // the view counts it already, as it does before a wallet exists.
        assert_eq!(view["balance_mc"], 5_000 + 5_000_000);
        let entry = &view["ledger"][0];
        assert_eq!(entry["kind"], "grant");
        assert_eq!(entry["delta_mc"], 5_000);
        assert_eq!(entry["balance_after_mc"], 5_000);
        assert_eq!(entry["note"], "welcome bonus");
        assert_eq!(entry["by"], admin_email.as_str());
        let who: Value =
            sqlx::query_scalar("SELECT metadata->'admin' FROM credit_ledger WHERE account_id = $1")
                .bind(account)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(who["user_id"], admin_id.to_string());

        // Members see the entry and its note, not who made it.
        let view = member_view(&pool, &member, account).await;
        assert_eq!(view["ledger"][0]["note"], "welcome bonus");
        assert!(view["ledger"][0].get("by").is_none());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn adjustments_go_either_way_and_need_a_note(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        wallet(&pool, account, 1_000, true).await;
        let (_, admin_token, _) = admin(&pool).await;
        let path = format!("/v1/admin/accounts/{account}/credits/ledger");

        let (status, _) = call(
            &pool,
            Method::POST,
            &path,
            &admin_token,
            Some(json!({ "kind": "adjustment", "amount_mc": -300 })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "an adjustment needs a note"
        );

        let (status, view) = call(
            &pool,
            Method::POST,
            &path,
            &admin_token,
            Some(json!({ "kind": "adjustment", "amount_mc": -300, "note": "refunded order 42" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["balance_mc"], 700);
        assert_eq!(view["ledger"][0]["kind"], "adjustment");
        assert_eq!(view["ledger"][0]["delta_mc"], -300);
        assert_eq!(view["ledger"][0]["balance_after_mc"], 700);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn bad_ledger_requests_change_nothing(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        let (_, admin_token, _) = admin(&pool).await;
        let path = format!("/v1/admin/accounts/{account}/credits/ledger");
        let bad = [
            json!({ "kind": "grant", "amount_mc": 0 }),
            json!({ "kind": "grant", "amount_mc": -5 }),
            json!({ "kind": "adjustment", "amount_mc": 0, "note": "x" }),
            json!({ "kind": "refund", "amount_mc": 100, "note": "x" }),
            json!({ "kind": "grant", "amount_mc": 2_000_000_000_000_i64 }),
            json!({ "kind": "grant", "amount_mc": 100, "note": "x".repeat(501) }),
        ];
        for body in bad {
            let (status, _) =
                call(&pool, Method::POST, &path, &admin_token, Some(body.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        let (status, _) = call(
            &pool,
            Method::POST,
            &format!("/v1/admin/accounts/{}/credits/ledger", Uuid::now_v7()),
            &admin_token,
            Some(json!({ "kind": "grant", "amount_mc": 100 })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn settings_override_and_reset_with_history(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        let (_, admin_token, _) = admin(&pool).await;
        let path = format!("/v1/admin/accounts/{account}/credits");

        let (status, view) = call(
            &pool,
            Method::PATCH,
            &path,
            &admin_token,
            Some(json!({
                "rate_limit_per_min": 120,
                "monthly_free_grant_mc": 250_000,
                "unlimited": true,
                "note": "design partner"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["rate_limit_per_min"], 120);
        assert_eq!(view["rate_limit_override_per_min"], 120);
        assert_eq!(view["monthly_free_grant_mc"], 250_000);
        assert_eq!(view["status"], "unlimited");
        let entry = &view["ledger"][0];
        assert_eq!(entry["kind"], "settings");
        assert_eq!(entry["delta_mc"], 0);
        assert_eq!(
            entry["changes"]["rate_limit_per_min"],
            json!({ "from": null, "to": 120 })
        );
        assert_eq!(
            entry["changes"]["unlimited"],
            json!({ "from": false, "to": true })
        );
        assert_eq!(entry["note"], "design partner");

        // `null` resets to the default; absent settings stay as they are.
        let (_, view) = call(
            &pool,
            Method::PATCH,
            &path,
            &admin_token,
            Some(json!({ "rate_limit_per_min": null })),
        )
        .await;
        assert_eq!(view["rate_limit_per_min"], 60);
        assert_eq!(view["rate_limit_override_per_min"], Value::Null);
        assert_eq!(view["monthly_free_grant_mc"], 250_000);
        assert_eq!(view["unlimited"], true);

        // Setting what's already set records nothing.
        let (_, view) = call(
            &pool,
            Method::PATCH,
            &path,
            &admin_token,
            Some(json!({ "unlimited": true })),
        )
        .await;
        assert_eq!(view["ledger"].as_array().unwrap().len(), 2);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn bad_settings_are_rejected(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        let (_, admin_token, _) = admin(&pool).await;
        let path = format!("/v1/admin/accounts/{account}/credits");
        for body in [
            json!({ "rate_limit_per_min": 0 }),
            json!({ "rate_limit_per_min": -1 }),
            json!({ "monthly_free_grant_mc": -1 }),
        ] {
            let (status, _) = call(
                &pool,
                Method::PATCH,
                &path,
                &admin_token,
                Some(body.clone()),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn balances_still_reconcile_after_admin_changes(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        // A month's grant and 3 charges, as the worker would leave them.
        wallet(&pool, account, 100_000 - 300, true).await;
        sqlx::query(
            "INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc)
             VALUES ($1, 100000, 'free_grant', 100000)",
        )
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
        charge(&pool, account, 3).await;

        let (_, admin_token, _) = admin(&pool).await;
        let base = format!("/v1/admin/accounts/{account}/credits");
        for (method, path, body) in [
            (
                Method::POST,
                format!("{base}/ledger"),
                json!({ "kind": "grant", "amount_mc": 5_000 }),
            ),
            (
                Method::POST,
                format!("{base}/ledger"),
                json!({ "kind": "adjustment", "amount_mc": -1_200, "note": "correction" }),
            ),
            (
                Method::PATCH,
                base.clone(),
                json!({ "rate_limit_per_min": 30 }),
            ),
        ] {
            let (status, view) = call(&pool, method, &path, &admin_token, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{view}");
        }

        let (balance, reconciled): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT balance_mc FROM credit_wallets WHERE account_id = $1),
                    (COALESCE((SELECT SUM(delta_mc) FROM credit_ledger WHERE account_id = $1), 0)
                   - COALESCE((SELECT SUM(cost_mc) FROM invocation_charges WHERE account_id = $1), 0))::bigint",
        )
        .bind(account)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(balance, 100_000 - 300 + 5_000 - 1_200);
        assert_eq!(balance, reconciled);
    }

    /// A pool with room for real concurrency (the test pool may be small).
    async fn wide_pool(pool: &PgPool) -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap()
    }

    /// Consent to an OAuth client as `session`, then redeem the code: the
    /// access token a third-party client would hold.
    async fn oauth_client_token(pool: &PgPool, session: &str, agent: Uuid) -> String {
        use base64::Engine;
        use sha2::{Digest, Sha256};

        sqlx::query(
            "INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris)
             VALUES ($1, 'mcp_test', 'Test', $2)",
        )
        .bind(Uuid::now_v7())
        .bind(vec!["https://app.test/cb".to_string()])
        .execute(pool)
        .await
        .unwrap();
        let verifier = "verifier-0123456789012345678901234567890123456789";
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let (status, issued) = call(
            pool,
            Method::POST,
            "/oauth/issue-code",
            session,
            Some(json!({
                "client_id": "mcp_test",
                "redirect_uri": "https://app.test/cb",
                "code_challenge": challenge,
                "agent_scope": "selected",
                "selected_agent_ids": [agent],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{issued}");
        let form = format!(
            "grant_type=authorization_code&code={}&client_id=mcp_test\
             &redirect_uri=https%3A%2F%2Fapp.test%2Fcb&code_verifier={verifier}",
            issued["code"].as_str().unwrap()
        );
        let res = crate::router(crate::AppState::new(pool.clone(), test_config()))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        body["access_token"].as_str().unwrap().to_owned()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn delegated_credentials_never_act_as_admin(pool: PgPool) {
        let (_, _, victim) = seed_user_with_personal(&pool, "victim").await;
        // The operator: an admin in the database, signed in interactively.
        let (admin_id, session, email) = admin(&pool).await;
        sqlx::query("UPDATE users SET is_admin = true WHERE id = $1")
            .bind(admin_id)
            .execute(&pool)
            .await
            .unwrap();
        let admin_account: Uuid =
            sqlx::query_scalar("SELECT account_id FROM account_memberships WHERE user_id = $1")
                .bind(admin_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let agent = seed_agent(&pool, admin_account, "ops-agent", admin_id).await;
        let ledger = format!("/v1/admin/accounts/{victim}/credits/ledger");
        let grant = json!({ "kind": "grant", "amount_mc": 1_000_000 });

        // A token the operator consented to for an OAuth client, even one
        // scoped to a single agent.
        let client_token = oauth_client_token(&pool, &session, agent).await;
        let claims = jwt::decode_jwt(&client_token, TEST_SECRET).unwrap();
        assert!(
            !claims.is_admin,
            "delegated tokens are minted without admin"
        );

        // A delegated token minted before that, still carrying the flag: its
        // jti is on the pairing that minted it.
        let pairing = seed_approved_device_flow(&pool, admin_id, agent).await;
        let old = jwt::UserClaims::new(admin_id, email, true, 1);
        sqlx::query("UPDATE oauth_device_codes SET minted_jti = $1 WHERE id = $2")
            .bind(old.jti)
            .bind(pairing)
            .execute(&pool)
            .await
            .unwrap();
        let old_token = jwt::encode_jwt(&old, TEST_SECRET).unwrap();

        // The operator's API key.
        let (_, api_key) = seed_api_key(&pool, admin_id, "ops").await;

        for (who, token) in [
            ("oauth client", &client_token),
            ("pre-fix pairing token", &old_token),
            ("api key", &api_key),
        ] {
            let (status, _) = call(&pool, Method::POST, &ledger, token, Some(grant.clone())).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} moved credits");
            let (status, _) = call(&pool, Method::GET, "/v1/admin/orgs", token, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} read the admin list");
        }
        let wallets: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_wallets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(wallets, 0);

        // The operator's own session still works.
        let (status, view) = call(&pool, Method::POST, &ledger, &session, Some(grant)).await;
        assert_eq!(status, StatusCode::OK, "{view}");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn concurrent_first_settings_changes_both_stick(pool: PgPool) {
        let (_, admin_token, _) = admin(&pool).await;
        let wide = wide_pool(&pool).await;
        for run in 0..5 {
            let (_, _, account) = seed_user_with_personal(&pool, &format!("race{run}")).await;
            let path = format!("/v1/admin/accounts/{account}/credits");
            let (grant, rate) = tokio::join!(
                call(
                    &wide,
                    Method::PATCH,
                    &path,
                    &admin_token,
                    Some(json!({ "monthly_free_grant_mc": 5_000 })),
                ),
                call(
                    &wide,
                    Method::PATCH,
                    &path,
                    &admin_token,
                    Some(json!({ "rate_limit_per_min": 10 })),
                ),
            );
            assert_eq!((grant.0, rate.0), (StatusCode::OK, StatusCode::OK));
            let stored: (Option<i64>, Option<i32>) = sqlx::query_as(
                "SELECT monthly_free_grant_mc, rate_limit_per_min FROM credit_wallets WHERE account_id = $1",
            )
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                stored,
                (Some(5_000), Some(10)),
                "run {run}: a change was lost"
            );
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn the_admin_account_list_shows_credit_status(pool: PgPool) {
        let (_, _, fresh) = seed_user_with_personal(&pool, "fresh").await;
        let (_, _, broke) = seed_user_with_personal(&pool, "broke").await;
        wallet(&pool, broke, 50, true).await;
        let (_, _, partner) = seed_user_with_personal(&pool, "partner").await;
        wallet(&pool, partner, -500, true).await;
        sqlx::query("UPDATE credit_wallets SET unlimited = true WHERE account_id = $1")
            .bind(partner)
            .execute(&pool)
            .await
            .unwrap();

        let (_, admin_token, _) = admin(&pool).await;
        let (status, orgs) = call(&pool, Method::GET, "/v1/admin/orgs", &admin_token, None).await;
        assert_eq!(status, StatusCode::OK, "{orgs}");
        let row = |id: Uuid| {
            orgs.as_array()
                .unwrap()
                .iter()
                .find(|o| o["id"] == id.to_string())
                .unwrap()
                .clone()
        };
        assert_eq!(row(fresh)["credit_balance_mc"], Value::Null);
        assert_eq!(row(fresh)["credit_status"], "active");
        assert_eq!(row(broke)["credit_balance_mc"], 50);
        assert_eq!(row(broke)["credit_status"], "blocked");
        assert_eq!(row(partner)["credit_status"], "unlimited");
    }
}
