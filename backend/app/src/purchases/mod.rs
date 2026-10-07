//! Buying credits through Dodo Payments (credits P4), on chakramcp.com only.
//! Spec: `docs/superpowers/specs/2026-10-06-credits-p4-purchasing-design.md`.
//!
//! Our server records every checkout before the buyer sees Dodo's, and only
//! Dodo's signed webhook credits a payment: matched by its checkout session,
//! checked against what we asked for, and applied exactly once. Like every
//! other credit change, the wallet and its ledger row move in one
//! transaction, and the relay sees the new balance at its next switch
//! refresh. Nothing here touches the invocation path.

pub mod config;
pub mod dodo;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

pub use config::{PurchaseConfig, PurchaseInfo};
use dodo::{NewCheckout, Payment};

/// Payments returned with a credits view, newest first.
pub const PAYMENTS_LIMIT: i64 = 50;
/// Checkouts an account may create per rolling hour.
pub const CHECKOUTS_PER_HOUR: i64 = 10;
const SET_LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

/// A checkout as the payments list shows it. `status` is `open`, `paid`,
/// `failed` or `unapplied`, or `expired` for an `open` one over a day old.
#[derive(Debug, Serialize)]
pub struct PaymentView {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub amount_cents: i32,
    pub currency: String,
    pub credits_mc: i64,
    pub status: String,
    pub buyer_email: String,
    pub invoice_url: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CreatedCheckout {
    pub checkout_id: Uuid,
    pub checkout_url: String,
    pub credits_mc: i64,
}

#[derive(Debug, Serialize)]
pub struct CheckoutStatus {
    pub id: Uuid,
    pub status: String,
    pub amount_cents: i32,
    pub credits_mc: i64,
    pub created_at: DateTime<Utc>,
    pub paid_at: Option<DateTime<Utc>>,
}

pub struct Buyer<'a> {
    pub user_id: Uuid,
    pub email: &'a str,
    pub name: Option<&'a str>,
}

/// Record a checkout, then open a Dodo session for it.
pub async fn create_checkout(
    db: &PgPool,
    cfg: &PurchaseConfig,
    frontend_base_url: &str,
    account_id: Uuid,
    account_slug: &str,
    buyer: Buyer<'_>,
    amount_cents: i64,
) -> ApiResult<CreatedCheckout> {
    let amount_cents = i32::try_from(amount_cents)
        .ok()
        .filter(|c| (cfg.min_cents..=cfg.max_cents).contains(c))
        .ok_or_else(|| {
            ApiError::InvalidRequest(format!(
                "amount_cents must be between {} and {}",
                cfg.min_cents, cfg.max_cents
            ))
        })?;
    let recent = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM credit_checkouts
            WHERE account_id = $1 AND created_at > now() - interval '1 hour'"#,
        account_id,
    )
    .fetch_one(db)
    .await?;
    if recent >= CHECKOUTS_PER_HOUR {
        return Err(ApiError::TooManyCheckouts);
    }

    let id = Uuid::now_v7();
    let credits_mc = cfg.credits_mc(amount_cents);
    sqlx::query!(
        r#"
        INSERT INTO credit_checkouts
            (id, account_id, user_id, buyer_email, amount_cents, credits_mc)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
        id,
        account_id,
        buyer.user_id,
        buyer.email,
        amount_cents,
        credits_mc,
    )
    .execute(db)
    .await?;

    let return_url = format!(
        "{}/app/credits?account={}&checkout={id}",
        frontend_base_url.trim_end_matches('/'),
        urlencoding::encode(account_slug),
    );
    let checkout = NewCheckout {
        product_id: &cfg.product_id,
        amount_cents,
        email: buyer.email,
        name: buyer.name,
        return_url: &return_url,
        checkout_id: id,
        account_slug,
    };
    let session = match cfg.client.create_checkout(&checkout).await {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!(
                event = "credits.checkout_failed",
                %account_id,
                checkout_id = %id,
                error = %e,
                "Dodo couldn't open a checkout"
            );
            sqlx::query!(
                "UPDATE credit_checkouts SET status = 'failed', updated_at = now() WHERE id = $1",
                id
            )
            .execute(db)
            .await?;
            return Err(ApiError::PaymentProvider);
        }
    };
    sqlx::query!(
        "UPDATE credit_checkouts SET dodo_session_id = $2, updated_at = now() WHERE id = $1",
        id,
        session.session_id,
    )
    .execute(db)
    .await?;
    tracing::info!(
        event = "credits.checkout_created",
        %account_id,
        checkout_id = %id,
        amount_cents,
        "checkout created"
    );
    Ok(CreatedCheckout {
        checkout_id: id,
        checkout_url: session.checkout_url,
        credits_mc,
    })
}

/// One of the account's checkouts; another account's is `NotFound`.
pub async fn checkout_status(db: &PgPool, account_id: Uuid, id: Uuid) -> ApiResult<CheckoutStatus> {
    sqlx::query_as!(
        CheckoutStatus,
        r#"
        SELECT id, amount_cents, credits_mc, created_at, paid_at,
               CASE WHEN status = 'open' AND created_at < now() - interval '24 hours'
                    THEN 'expired' ELSE status END AS "status!"
          FROM credit_checkouts WHERE id = $1 AND account_id = $2
        "#,
        id,
        account_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)
}

/// The account's latest checkouts, for the credits view.
pub async fn recent_payments(
    tx: &mut Transaction<'_, Postgres>,
    account_id: Uuid,
) -> ApiResult<Vec<PaymentView>> {
    Ok(sqlx::query_as!(
        PaymentView,
        r#"
        SELECT id, created_at, amount_cents, currency, credits_mc, buyer_email,
               invoice_url, paid_at,
               CASE WHEN status = 'open' AND created_at < now() - interval '24 hours'
                    THEN 'expired' ELSE status END AS "status!"
          FROM credit_checkouts WHERE account_id = $1
         ORDER BY created_at DESC, id DESC
         LIMIT $2
        "#,
        account_id,
        PAYMENTS_LIMIT,
    )
    .fetch_all(&mut **tx)
    .await?)
}

/// What a `payment.succeeded` webhook did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Paid {
        account_id: Uuid,
        checkout_id: Uuid,
        credits_mc: i64,
    },
    /// A redelivery of a payment already handled.
    Duplicate,
    /// A payment for credits we can't tie to one of our checkouts: no
    /// session, an unknown one, or a second payment on a settled one.
    Unmatched(&'static str),
    /// Another product on the same Dodo account: not ours to handle.
    OtherProduct,
    /// Paid but not credited, for the operator (`account_gone`,
    /// `not_as_agreed`).
    Unapplied {
        checkout_id: Uuid,
        reason: &'static str,
    },
}

/// Apply a `payment.succeeded` (spec §6.2), in one transaction.
pub async fn apply_succeeded(
    db: &PgPool,
    cfg: &PurchaseConfig,
    payment: &Payment,
) -> ApiResult<Outcome> {
    let mut tx = db.begin().await?;
    sqlx::query(SET_LOCK_TIMEOUT).execute(&mut *tx).await?;
    let row = match &payment.checkout_session_id {
        Some(session) => {
            sqlx::query!(
                r#"
                SELECT id, account_id, user_id, buyer_email, amount_cents, currency,
                       credits_mc, status, dodo_payment_id
                  FROM credit_checkouts WHERE dodo_session_id = $1
                   FOR UPDATE
                "#,
                session,
            )
            .fetch_optional(&mut *tx)
            .await?
        }
        None => None,
    };
    let Some(row) = row else {
        tx.rollback().await?;
        // A cart naming only other products belongs to another integration;
        // anything else (our product, or no cart at all) needs a look.
        let others_only = payment.product_cart.as_ref().is_some_and(|cart| {
            !cart.is_empty() && cart.iter().all(|l| l.product_id != cfg.product_id)
        });
        return Ok(if others_only {
            Outcome::OtherProduct
        } else if payment.checkout_session_id.is_none() {
            Outcome::Unmatched("no checkout session")
        } else {
            Outcome::Unmatched("unknown checkout session")
        });
    };
    if row.status == "paid" || row.status == "unapplied" {
        tx.rollback().await?;
        return Ok(
            if row.dodo_payment_id.as_deref() == Some(payment.payment_id.as_str()) {
                Outcome::Duplicate
            } else {
                Outcome::Unmatched("a second payment on a settled checkout")
            },
        );
    }
    if let Some(claimed) = payment
        .metadata
        .as_ref()
        .and_then(|m| m.get("checkout_id"))
        .and_then(Value::as_str)
    {
        if claimed != row.id.to_string() {
            tracing::warn!(
                checkout_id = %row.id,
                claimed,
                payment_id = %payment.payment_id,
                "the payment's metadata names another checkout; matched by session"
            );
        }
    }

    let as_agreed = payment.currency.as_deref() == Some(row.currency.as_str())
        && payment
            .total_amount
            .is_some_and(|total| total >= i64::from(row.amount_cents))
        && payment.discounts.as_ref().is_none_or(Vec::is_empty);
    let account_exists = sqlx::query_scalar!(
        r#"SELECT 1 AS "one!" FROM accounts WHERE id = $1"#,
        row.account_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    let unapplied = if !as_agreed {
        Some("not_as_agreed")
    } else if !account_exists {
        Some("account_gone")
    } else {
        None
    };
    if let Some(reason) = unapplied {
        let updated = sqlx::query!(
            r#"
            UPDATE credit_checkouts
               SET status = 'unapplied', unapplied_reason = $2, dodo_payment_id = $3,
                   invoice_url = $4, updated_at = now()
             WHERE id = $1
            "#,
            row.id,
            reason,
            payment.payment_id,
            payment.invoice_url,
        )
        .execute(&mut *tx)
        .await;
        if is_unique_violation(&updated) {
            tx.rollback().await?;
            return Ok(Outcome::Unmatched(
                "a payment id already recorded on another checkout",
            ));
        }
        updated?;
        tx.commit().await?;
        return Ok(Outcome::Unapplied {
            checkout_id: row.id,
            reason,
        });
    }

    let balance_mc = sqlx::query_scalar!(
        r#"
        INSERT INTO credit_wallets (account_id, balance_mc, updated_at)
        VALUES ($1, $2, now())
        ON CONFLICT (account_id) DO UPDATE
           SET balance_mc = credit_wallets.balance_mc + EXCLUDED.balance_mc,
               updated_at = now()
        RETURNING balance_mc
        "#,
        row.account_id,
        row.credits_mc,
    )
    .fetch_one(&mut *tx)
    .await?;
    let metadata = json!({
        "kind": "purchase",
        "checkout_id": row.id,
        "amount_cents": row.amount_cents,
        "currency": row.currency,
        "buyer": { "user_id": row.user_id, "email": row.buyer_email },
        "dodo": {
            "total_amount": payment.total_amount,
            "tax": payment.tax,
            "currency": payment.currency,
        },
    });
    // A plain INSERT: `credit_ledger_purchase_ref_uniq` turns a second
    // credit for this payment into an error that rolls back the wallet too.
    let ledger = sqlx::query!(
        r#"
        INSERT INTO credit_ledger
            (account_id, delta_mc, reason, external_ref, balance_after_mc, metadata)
        VALUES ($1, $2, 'purchase', $3, $4, $5)
        "#,
        row.account_id,
        row.credits_mc,
        payment.payment_id,
        balance_mc,
        metadata,
    )
    .execute(&mut *tx)
    .await;
    if is_unique_violation(&ledger) {
        tx.rollback().await?;
        return Ok(Outcome::Unmatched("a payment id already credited"));
    }
    ledger?;
    let paid = sqlx::query!(
        r#"
        UPDATE credit_checkouts
           SET status = 'paid', unapplied_reason = NULL, dodo_payment_id = $2,
               invoice_url = $3, paid_at = now(), updated_at = now()
         WHERE id = $1
        "#,
        row.id,
        payment.payment_id,
        payment.invoice_url,
    )
    .execute(&mut *tx)
    .await;
    if is_unique_violation(&paid) {
        tx.rollback().await?;
        return Ok(Outcome::Unmatched(
            "a payment id already recorded on another checkout",
        ));
    }
    paid?;
    tx.commit().await?;
    Ok(Outcome::Paid {
        account_id: row.account_id,
        checkout_id: row.id,
        credits_mc: row.credits_mc,
    })
}

/// Apply a `payment.failed` or `payment.cancelled`: an open checkout becomes
/// `failed`. Returns whether one did.
pub async fn apply_failed(db: &PgPool, payment: &Payment) -> ApiResult<bool> {
    let Some(session) = &payment.checkout_session_id else {
        return Ok(false);
    };
    let done = sqlx::query!(
        r#"
        UPDATE credit_checkouts SET status = 'failed', updated_at = now()
         WHERE dodo_session_id = $1 AND status = 'open'
        "#,
        session,
    )
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

fn is_unique_violation<T>(result: &Result<T, sqlx::Error>) -> bool {
    matches!(result, Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505"))
}
