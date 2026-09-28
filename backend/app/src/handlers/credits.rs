//! Credits for people: what an account has and spends (owners, P2) and how
//! the operator manages it (admins, P3). Design:
//! `docs/specs/2026-09-23-credit-ledger-foundation-design.md`.
//!
//! All of this is off the invocation path. Balances only move through the
//! ledger: every admin write is one statement that updates the wallet and
//! records a `credit_ledger` row carrying the resulting balance, so
//! `balance = Σ ledger − Σ charges` keeps holding. A block or unblock takes
//! effect at the relay's next switch refresh (one tick, 5 s by default).

use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::credits::{is_blocked, CreditsConfig};
use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{AdminUser, AuthUser};
use crate::state::AppState;

/// Ledger rows returned with a view, newest first.
const LEDGER_LIMIT: i64 = 50;
/// Largest single grant, adjustment or monthly-grant override: a billion credits.
const MAX_AMOUNT_MC: i64 = 1_000_000_000_000;
const MAX_RATE_LIMIT_PER_MIN: i32 = 1_000_000;
const MAX_NOTE_CHARS: usize = 500;
/// Admin writes wait at most this long for a wallet row the worker holds.
const SET_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

#[derive(Debug, Serialize)]
pub struct CreditsView {
    pub account_id: Uuid,
    /// Milli-credits (1 credit = 1,000). Can be slightly negative: charging
    /// is asynchronous. With no wallet yet, the grant the first invocation
    /// brings.
    pub balance_mc: i64,
    /// False until the account's first charge or an admin change.
    pub has_wallet: bool,
    /// `active`, `blocked` (out of credits) or `unlimited`.
    pub status: &'static str,
    pub cost_per_invocation_mc: i64,
    /// The monthly free grant in effect, and the override behind it
    /// (`None` = the global default).
    pub monthly_free_grant_mc: i64,
    pub monthly_free_grant_override_mc: Option<i64>,
    /// The rate limit in effect, and the override behind it.
    pub rate_limit_per_min: i32,
    pub rate_limit_override_per_min: Option<i32>,
    pub unlimited: bool,
    /// First of the month last granted; `None` = never granted.
    pub last_grant_period: Option<NaiveDate>,
    /// When the next free grant lands (the first of next month, UTC).
    pub next_grant_on: NaiveDate,
    pub spent_this_month_mc: i64,
    pub invocations_this_month: i64,
    /// Charged invocations per UTC day over the last 30 days (active days only).
    pub daily: Vec<DailySpend>,
    /// Newest first.
    pub ledger: Vec<LedgerEntry>,
}

#[derive(Debug, Serialize)]
pub struct DailySpend {
    pub day: NaiveDate,
    pub invocations: i64,
    pub spent_mc: i64,
}

#[derive(Debug, Serialize)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    /// `free_grant`, `grant`, `adjustment`, `settings` or `purchase`.
    pub kind: String,
    pub delta_mc: i64,
    pub balance_after_mc: i64,
    pub note: Option<String>,
    /// Free grants: the month granted (its first day).
    pub period: Option<String>,
    /// Settings changes: `{setting: {from, to}}`, `null` meaning the default.
    pub changes: Option<Value>,
    /// Admin view only: who made the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

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
    Ok(Json(
        load_view(&state.db, &state.credits, account_id, false).await?,
    ))
}

// ─────────────────────────────────────────────────────────
// GET /v1/admin/accounts/{account_id}/credits
// ─────────────────────────────────────────────────────────
pub async fn admin_account_credits(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(account_id): Path<Uuid>,
) -> ApiResult<Json<CreditsView>> {
    ensure_account(&state.db, account_id).await?;
    Ok(Json(
        load_view(&state.db, &state.credits, account_id, true).await?,
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
    let note = clean_note(req.note)?;
    let reason = match req.kind.as_str() {
        "grant" if req.amount_mc > 0 => "admin_grant",
        "grant" => return Err(invalid("a grant must be a positive amount")),
        "adjustment" if req.amount_mc == 0 => {
            return Err(invalid("an adjustment must be non-zero"))
        }
        "adjustment" if note.is_none() => return Err(invalid("an adjustment needs a note")),
        "adjustment" => "adjustment",
        _ => return Err(invalid("kind must be `grant` or `adjustment`")),
    };
    if req.amount_mc.unsigned_abs() > MAX_AMOUNT_MC as u64 {
        return Err(invalid("amount is too large"));
    }
    ensure_account(&state.db, account_id).await?;

    let metadata = json!({
        "kind": req.kind,
        "note": note,
        "admin": { "user_id": admin.user_id, "email": admin.email },
    });
    let mut tx = state.db.begin().await?;
    sqlx::query(SET_LOCK_TIMEOUT).execute(&mut *tx).await?;
    // A wallet created here has no grant period yet, so the worker also
    // applies the account's monthly free grant on its next pass.
    sqlx::query!(
        r#"
        WITH w AS (
            INSERT INTO credit_wallets (account_id, balance_mc, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (account_id) DO UPDATE
               SET balance_mc = credit_wallets.balance_mc + EXCLUDED.balance_mc,
                   updated_at = now()
            RETURNING account_id, balance_mc
        )
        INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc, metadata)
        SELECT account_id, $2, $3, balance_mc, $4 FROM w
        "#,
        account_id,
        req.amount_mc,
        reason,
        metadata,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        event = "credits.admin_entry",
        %account_id,
        admin = %admin.email,
        reason,
        amount_mc = req.amount_mc,
        "admin credit entry"
    );
    Ok(Json(
        load_view(&state.db, &state.credits, account_id, true).await?,
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
    if let Some(Some(grant)) = req.monthly_free_grant_mc {
        if !(0..=MAX_AMOUNT_MC).contains(&grant) {
            return Err(invalid(
                "the monthly grant must be between 0 and a billion credits",
            ));
        }
    }
    if let Some(Some(rate)) = req.rate_limit_per_min {
        if !(1..=MAX_RATE_LIMIT_PER_MIN).contains(&rate) {
            return Err(invalid("the rate limit must be at least 1 per minute"));
        }
    }
    let note = clean_note(req.note)?;
    ensure_account(&state.db, account_id).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query(SET_LOCK_TIMEOUT).execute(&mut *tx).await?;
    let current = sqlx::query!(
        r#"
        SELECT monthly_free_grant_mc, rate_limit_per_min, unlimited
          FROM credit_wallets WHERE account_id = $1
           FOR UPDATE
        "#,
        account_id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    let (grant, rate, unlimited) = current
        .map(|w| (w.monthly_free_grant_mc, w.rate_limit_per_min, w.unlimited))
        .unwrap_or((None, None, false));
    let new_grant = req.monthly_free_grant_mc.unwrap_or(grant);
    let new_rate = req.rate_limit_per_min.unwrap_or(rate);
    let new_unlimited = req.unlimited.unwrap_or(unlimited);

    let mut changes = Map::new();
    if new_grant != grant {
        changes.insert(
            "monthly_free_grant_mc".into(),
            json!({ "from": grant, "to": new_grant }),
        );
    }
    if new_rate != rate {
        changes.insert(
            "rate_limit_per_min".into(),
            json!({ "from": rate, "to": new_rate }),
        );
    }
    if new_unlimited != unlimited {
        changes.insert(
            "unlimited".into(),
            json!({ "from": unlimited, "to": new_unlimited }),
        );
    }
    if changes.is_empty() {
        tx.rollback().await?;
    } else {
        let changes = Value::Object(changes);
        let metadata = json!({
            "kind": "settings",
            "note": note,
            "changes": changes,
            "admin": { "user_id": admin.user_id, "email": admin.email },
        });
        // The ledger records settings changes too (delta 0), so an
        // account's history explains every change to its limits.
        sqlx::query!(
            r#"
            WITH w AS (
                INSERT INTO credit_wallets
                    (account_id, monthly_free_grant_mc, rate_limit_per_min, unlimited, updated_at)
                VALUES ($1, $2, $3, $4, now())
                ON CONFLICT (account_id) DO UPDATE
                   SET monthly_free_grant_mc = EXCLUDED.monthly_free_grant_mc,
                       rate_limit_per_min = EXCLUDED.rate_limit_per_min,
                       unlimited = EXCLUDED.unlimited,
                       updated_at = now()
                RETURNING account_id, balance_mc
            )
            INSERT INTO credit_ledger (account_id, delta_mc, reason, balance_after_mc, metadata)
            SELECT account_id, 0, 'adjustment', balance_mc, $5 FROM w
            "#,
            account_id,
            new_grant,
            new_rate,
            new_unlimited,
            metadata,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        tracing::info!(
            event = "credits.admin_settings",
            %account_id,
            admin = %admin.email,
            %changes,
            "admin credit settings"
        );
    }
    Ok(Json(
        load_view(&state.db, &state.credits, account_id, true).await?,
    ))
}

/// The account's credit picture. `admin` adds who made each change.
pub(crate) async fn load_view(
    db: &PgPool,
    cfg: &CreditsConfig,
    account_id: Uuid,
    admin: bool,
) -> ApiResult<CreditsView> {
    let wallet = sqlx::query!(
        r#"
        SELECT balance_mc, monthly_free_grant_mc, rate_limit_per_min,
               free_grant_period, unlimited
          FROM credit_wallets WHERE account_id = $1
        "#,
        account_id,
    )
    .fetch_optional(db)
    .await?;
    // Month boundaries on the database clock, which stamps `charged_at`.
    let month = sqlx::query!(
        r#"
        WITH bounds AS (SELECT date_trunc('month', now() AT TIME ZONE 'UTC') AS start)
        SELECT (bounds.start + interval '1 month')::date AS "next_grant_on!",
               count(c.invocation_id) AS "invocations!",
               COALESCE(SUM(c.cost_mc), 0)::bigint AS "spent_mc!"
          FROM bounds
          LEFT JOIN invocation_charges c
            ON c.account_id = $1 AND c.charged_at >= bounds.start AT TIME ZONE 'UTC'
         GROUP BY bounds.start
        "#,
        account_id,
    )
    .fetch_one(db)
    .await?;
    let daily = sqlx::query_as!(
        DailySpend,
        r#"
        SELECT (charged_at AT TIME ZONE 'UTC')::date AS "day!",
               count(*) AS "invocations!",
               SUM(cost_mc)::bigint AS "spent_mc!"
          FROM invocation_charges
         WHERE account_id = $1 AND charged_at >= now() - interval '30 days'
         GROUP BY 1 ORDER BY 1
        "#,
        account_id,
    )
    .fetch_all(db)
    .await?;
    let ledger = sqlx::query!(
        r#"
        SELECT id, created_at, reason, delta_mc, balance_after_mc, metadata
          FROM credit_ledger WHERE account_id = $1
         ORDER BY created_at DESC, id DESC
         LIMIT $2
        "#,
        account_id,
        LEDGER_LIMIT,
    )
    .fetch_all(db)
    .await?;

    let (balance_mc, grant_override, rate_override, period, unlimited) = match &wallet {
        Some(w) => (
            w.balance_mc,
            w.monthly_free_grant_mc,
            w.rate_limit_per_min,
            w.free_grant_period,
            w.unlimited,
        ),
        None => (
            grant_override_or_default(None, cfg),
            None,
            None,
            None,
            false,
        ),
    };
    let status = if unlimited {
        "unlimited"
    } else if is_blocked(
        balance_mc,
        period.is_some(),
        unlimited,
        cfg.cost_per_invocation_mc,
    ) {
        "blocked"
    } else {
        "active"
    };

    Ok(CreditsView {
        account_id,
        balance_mc,
        has_wallet: wallet.is_some(),
        status,
        cost_per_invocation_mc: cfg.cost_per_invocation_mc,
        monthly_free_grant_mc: grant_override_or_default(grant_override, cfg),
        monthly_free_grant_override_mc: grant_override,
        rate_limit_per_min: rate_override.unwrap_or(cfg.default_rate_per_min),
        rate_limit_override_per_min: rate_override,
        unlimited,
        last_grant_period: period,
        next_grant_on: month.next_grant_on,
        spent_this_month_mc: month.spent_mc,
        invocations_this_month: month.invocations,
        daily,
        ledger: ledger
            .into_iter()
            .map(|r| {
                ledger_entry(
                    r.id,
                    r.created_at,
                    &r.reason,
                    r.delta_mc,
                    r.balance_after_mc,
                    r.metadata,
                    admin,
                )
            })
            .collect(),
    })
}

fn grant_override_or_default(grant_override: Option<i64>, cfg: &CreditsConfig) -> i64 {
    grant_override.unwrap_or(cfg.default_monthly_free_mc)
}

fn ledger_entry(
    id: Uuid,
    created_at: DateTime<Utc>,
    reason: &str,
    delta_mc: i64,
    balance_after_mc: i64,
    metadata: Option<Value>,
    admin: bool,
) -> LedgerEntry {
    let meta = metadata.unwrap_or(Value::Null);
    let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_owned);
    let kind = match reason {
        "admin_grant" => "grant",
        "adjustment" if meta.get("kind").and_then(Value::as_str) == Some("settings") => "settings",
        other => other,
    };
    LedgerEntry {
        id,
        created_at,
        kind: kind.to_owned(),
        delta_mc,
        balance_after_mc,
        note: text("note"),
        period: text("period"),
        changes: meta.get("changes").cloned(),
        by: admin
            .then(|| {
                meta.pointer("/admin/email")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .flatten(),
    }
}

async fn ensure_account(db: &PgPool, account_id: Uuid) -> ApiResult<()> {
    sqlx::query_scalar!(
        r#"SELECT 1 AS "one!" FROM accounts WHERE id = $1"#,
        account_id
    )
    .fetch_optional(db)
    .await?
    .map(|_| ())
    .ok_or(ApiError::NotFound)
}

fn clean_note(note: Option<String>) -> ApiResult<Option<String>> {
    let note = note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    if note
        .as_ref()
        .is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS)
    {
        return Err(invalid("the note is too long (500 characters at most)"));
    }
    Ok(note)
}

fn invalid(message: &str) -> ApiError {
    ApiError::InvalidRequest(message.to_owned())
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
        assert_eq!(view["balance_mc"], 100_000);
        assert_eq!(view["status"], "active");
        assert_eq!(view["monthly_free_grant_mc"], 100_000);
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
        // The new wallet's monthly grant lands on the worker's next pass.
        assert_eq!(view["balance_mc"], 5_000);
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
