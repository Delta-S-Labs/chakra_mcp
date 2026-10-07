//! Reading and changing an account's credits, shared by the admin API
//! (`handlers::credits`) and the operator commands
//! (`chakramcp-server credits`). Design:
//! `docs/specs/2026-09-23-credit-ledger-foundation-design.md`.
//!
//! All of this is off the invocation path. Balances only move through the
//! ledger: every write is one statement that updates the wallet and records
//! a `credit_ledger` row carrying the resulting balance, so
//! `balance = Σ ledger − Σ charges` keeps holding. A block or unblock takes
//! effect at the relay's next switch refresh (one tick, 5 s by default).

use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::credits::{is_blocked, CreditsConfig};
use chakramcp_shared::error::{ApiError, ApiResult};

use crate::purchases::{self, PaymentView, PurchaseInfo};

/// Ledger rows returned with a view, newest first.
const LEDGER_LIMIT: i64 = 50;
/// Largest single grant, adjustment or monthly-grant override: a billion credits.
pub const MAX_AMOUNT_MC: i64 = 1_000_000_000_000;
pub const MAX_RATE_LIMIT_PER_MIN: i32 = 1_000_000;
const MAX_NOTE_CHARS: usize = 500;
/// Writes wait at most this long for a wallet row the worker holds.
const SET_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";
/// How views label changes made with `chakramcp-server credits`.
const OPERATOR_LABEL: &str = "operator (CLI)";

/// Who changes an account's credits, recorded on its ledger rows.
#[derive(Debug, Clone)]
pub enum Actor {
    /// The `ADMIN_EMAIL` user, signed in interactively (the admin API).
    Admin { user_id: Uuid, email: String },
    /// Someone on the host, through `chakramcp-server credits`.
    Operator,
}

impl Actor {
    /// Adds this actor to a ledger row's metadata.
    fn record(&self, metadata: &mut Value) {
        match self {
            Actor::Admin { user_id, email } => {
                metadata["admin"] = json!({ "user_id": user_id, "email": email });
            }
            Actor::Operator => metadata["operator"] = json!("cli"),
        }
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actor::Admin { email, .. } => f.write_str(email),
            Actor::Operator => f.write_str(OPERATOR_LABEL),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CreditsView {
    pub account_id: Uuid,
    /// Milli-credits (1 credit = 1,000). Can be slightly negative: charging
    /// is asynchronous. With no wallet yet, the grant the first invocation
    /// brings.
    pub balance_mc: i64,
    /// False until the account's first charge or an admin change.
    pub has_wallet: bool,
    /// Whether this server charges and enforces credits at all.
    pub enabled: bool,
    /// `active`, `blocked` (out of credits), `unlimited`, or `off` when
    /// credits are switched off on this server.
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
    /// The account's latest checkouts (credits P4), newest first.
    pub payments: Vec<PaymentView>,
    /// How to buy credits here; `None` where purchasing is off. Set by the
    /// members' endpoint only.
    pub purchase: Option<PurchaseInfo>,
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
    /// Purchases: what was paid and who bought. Every member sees it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purchase: Option<PurchaseLine>,
}

#[derive(Debug, Serialize)]
pub struct PurchaseLine {
    pub amount_cents: i64,
    pub currency: String,
    pub buyer_email: String,
}

/// Add a ledger entry. `kind` is `grant` (a positive amount) or `adjustment`
/// (either sign, with a note), as the admin API takes it.
pub async fn add_entry(
    db: &PgPool,
    account_id: Uuid,
    kind: &str,
    amount_mc: i64,
    note: Option<String>,
    actor: &Actor,
) -> ApiResult<()> {
    let note = clean_note(note)?;
    let reason = match kind {
        "grant" if amount_mc > 0 => "admin_grant",
        "grant" => return Err(invalid("a grant must be a positive amount")),
        "adjustment" if amount_mc == 0 => return Err(invalid("an adjustment must be non-zero")),
        "adjustment" if note.is_none() => return Err(invalid("an adjustment needs a note")),
        "adjustment" => "adjustment",
        _ => return Err(invalid("kind must be `grant` or `adjustment`")),
    };
    if amount_mc.unsigned_abs() > MAX_AMOUNT_MC as u64 {
        return Err(invalid("amount is too large"));
    }
    ensure_account(db, account_id).await?;

    let mut metadata = json!({ "kind": kind, "note": note });
    actor.record(&mut metadata);
    let mut tx = db.begin().await?;
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
        amount_mc,
        reason,
        metadata,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        event = "credits.admin_entry",
        %account_id,
        admin = %actor,
        reason,
        amount_mc,
        "admin credit entry"
    );
    Ok(())
}

/// A settings change. For the overrides, `None` leaves the setting as it is
/// and `Some(None)` puts it back to the default.
#[derive(Debug, Default)]
pub struct SettingsChange {
    pub monthly_free_grant_mc: Option<Option<i64>>,
    pub rate_limit_per_min: Option<Option<i32>>,
    pub unlimited: Option<bool>,
    pub note: Option<String>,
}

/// Apply a settings change, recorded as a zero-delta ledger row listing each
/// `{from, to}`. Setting what's already set records nothing.
pub async fn update_settings(
    db: &PgPool,
    account_id: Uuid,
    change: SettingsChange,
    actor: &Actor,
) -> ApiResult<()> {
    if let Some(Some(grant)) = change.monthly_free_grant_mc {
        if !(0..=MAX_AMOUNT_MC).contains(&grant) {
            return Err(invalid(
                "the monthly grant must be between 0 and a billion credits",
            ));
        }
    }
    if let Some(Some(rate)) = change.rate_limit_per_min {
        if !(1..=MAX_RATE_LIMIT_PER_MIN).contains(&rate) {
            return Err(invalid("the rate limit must be at least 1 per minute"));
        }
    }
    let note = clean_note(change.note)?;
    ensure_account(db, account_id).await?;

    let mut tx = db.begin().await?;
    sqlx::query(SET_LOCK_TIMEOUT).execute(&mut *tx).await?;
    // Make sure there's a row to lock: two first-time changes to a wallet-less
    // account would otherwise both read "no overrides" and the second write
    // would undo the first. (A no-op rolls this back.)
    sqlx::query!(
        "INSERT INTO credit_wallets (account_id) VALUES ($1) ON CONFLICT DO NOTHING",
        account_id,
    )
    .execute(&mut *tx)
    .await?;
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
    let new_grant = change.monthly_free_grant_mc.unwrap_or(grant);
    let new_rate = change.rate_limit_per_min.unwrap_or(rate);
    let new_unlimited = change.unlimited.unwrap_or(unlimited);

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
        return Ok(());
    }
    let changes = Value::Object(changes);
    let mut metadata = json!({ "kind": "settings", "note": note, "changes": changes });
    actor.record(&mut metadata);
    // The ledger records settings changes too (delta 0), so an account's
    // history explains every change to its limits.
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
        admin = %actor,
        %changes,
        "admin credit settings"
    );
    Ok(())
}

/// The account's credit picture, read from one snapshot so the balance,
/// spend and history agree. `admin` adds who made each change.
pub async fn load_view(
    db: &PgPool,
    cfg: &CreditsConfig,
    account_id: Uuid,
    admin: bool,
) -> ApiResult<CreditsView> {
    let mut tx = db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let wallet = sqlx::query!(
        r#"
        SELECT balance_mc, monthly_free_grant_mc, rate_limit_per_min,
               free_grant_period, unlimited
          FROM credit_wallets WHERE account_id = $1
        "#,
        account_id,
    )
    .fetch_optional(&mut *tx)
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
    .fetch_one(&mut *tx)
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
    .fetch_all(&mut *tx)
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
    .fetch_all(&mut *tx)
    .await?;
    let payments = purchases::recent_payments(&mut tx, account_id).await?;
    tx.commit().await?;

    let (balance_mc, grant_override, rate_override, period, unlimited) = match &wallet {
        Some(w) => (
            // A wallet made before the account's first grant (by a purchase
            // or an admin) gets that grant on the worker's next pass, seconds
            // away. Count it, as for an account with no wallet yet, so a first
            // purchase never shows a balance below the one before it.
            if cfg.enabled && w.free_grant_period.is_none() {
                w.balance_mc + grant_override_or_default(w.monthly_free_grant_mc, cfg)
            } else {
                w.balance_mc
            },
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
    let status = if !cfg.enabled {
        "off"
    } else if unlimited {
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
        enabled: cfg.enabled,
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
        payments,
        purchase: None,
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
    let by = || {
        meta.pointer("/admin/email")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| meta.get("operator").map(|_| OPERATOR_LABEL.to_owned()))
    };
    let purchase = (reason == "purchase").then(|| PurchaseLine {
        amount_cents: meta
            .get("amount_cents")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        currency: text("currency").unwrap_or_default(),
        buyer_email: meta
            .pointer("/buyer/email")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    });
    LedgerEntry {
        id,
        created_at,
        kind: kind.to_owned(),
        delta_mc,
        balance_after_mc,
        note: text("note"),
        period: text("period"),
        changes: meta.get("changes").cloned(),
        by: admin.then(by).flatten(),
        purchase,
    }
}

/// `NotFound` unless the account exists.
pub async fn ensure_account(db: &PgPool, account_id: Uuid) -> ApiResult<()> {
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
    use super::*;
    use crate::tests_support::seed_user_with_personal;

    #[sqlx::test(migrations = "../migrations")]
    async fn operator_changes_are_recorded_and_labelled(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        add_entry(
            &pool,
            account,
            "grant",
            5_000,
            Some("pilot".into()),
            &Actor::Operator,
        )
        .await
        .unwrap();
        update_settings(
            &pool,
            account,
            SettingsChange {
                unlimited: Some(true),
                ..Default::default()
            },
            &Actor::Operator,
        )
        .await
        .unwrap();

        let metadata: Vec<Value> = sqlx::query_scalar(
            "SELECT metadata FROM credit_ledger WHERE account_id = $1 ORDER BY created_at",
        )
        .bind(account)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(metadata.len(), 2);
        for m in &metadata {
            assert_eq!(m["operator"], "cli", "{m}");
            assert!(m.get("admin").is_none(), "{m}");
        }

        let cfg = CreditsConfig::default();
        let view = load_view(&pool, &cfg, account, true).await.unwrap();
        assert_eq!(view.status, "unlimited");
        // The new wallet's first monthly grant, seconds away, is counted.
        assert_eq!(view.balance_mc, 5_000 + cfg.default_monthly_free_mc);
        assert!(view
            .ledger
            .iter()
            .all(|e| e.by.as_deref() == Some(OPERATOR_LABEL)));
        // Members don't see who made a change.
        let view = load_view(&pool, &cfg, account, false).await.unwrap();
        assert!(view.ledger.iter().all(|e| e.by.is_none()));
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn with_credits_off_the_view_says_off(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        // A wallet left from a time credits were on, out of credits.
        sqlx::query(
            "INSERT INTO credit_wallets (account_id, balance_mc, free_grant_period)
             VALUES ($1, 50, date_trunc('month', now() AT TIME ZONE 'UTC')::date)",
        )
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
        let off = CreditsConfig {
            enabled: false,
            ..CreditsConfig::default()
        };
        let view = load_view(&pool, &off, account, true).await.unwrap();
        assert!(!view.enabled);
        assert_eq!(view.status, "off");
        assert_eq!(view.balance_mc, 50, "the balance is kept, not reset");

        let on = CreditsConfig::default();
        let view = load_view(&pool, &on, account, true).await.unwrap();
        assert!(view.enabled);
        assert_eq!(view.status, "blocked");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn the_operator_follows_the_same_rules(pool: PgPool) {
        let (_, _, account) = seed_user_with_personal(&pool, "customer").await;
        for (kind, amount, note) in [
            ("grant", 0, None),
            ("grant", -5, None),
            ("adjustment", -5, None),
            ("adjustment", 0, Some("x")),
            ("refund", 5, Some("x")),
            ("grant", MAX_AMOUNT_MC + 1, None),
        ] {
            let err = add_entry(
                &pool,
                account,
                kind,
                amount,
                note.map(str::to_owned),
                &Actor::Operator,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, ApiError::InvalidRequest(_)),
                "{kind} {amount}: {err:?}"
            );
        }
        assert!(matches!(
            add_entry(&pool, Uuid::now_v7(), "grant", 5, None, &Actor::Operator).await,
            Err(ApiError::NotFound)
        ));
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM credit_ledger")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }
}
