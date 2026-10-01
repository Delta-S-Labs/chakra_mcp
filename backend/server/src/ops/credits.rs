//! `chakramcp-server credits`: show and change an account's credits. Every
//! change is a ledger row recorded as made by the operator.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Subcommand, ValueEnum};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_app::accounts;
use chakramcp_app::credits_service::{self, Actor, CreditsView, SettingsChange};
use chakramcp_shared::credits::CreditsConfig;

use super::{connect, explain, format_credits, parse_credits, print_table};

#[derive(Subcommand, Debug)]
pub enum CreditsCmd {
    /// Show an account's balance, limits and recent ledger entries.
    Show {
        /// An account slug, an account id, or a user's email (their
        /// personal account).
        account: String,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Add credits to an account.
    Grant {
        account: String,
        /// Credits to add, e.g. 50 or 0.5.
        credits: String,
        /// Shown to the account's members with the entry.
        #[arg(long)]
        note: Option<String>,
    },
    /// Move an account's balance either way, e.g. to correct a mistake.
    Adjust {
        account: String,
        /// Credits to add, or take away when negative, e.g. -5.
        #[arg(allow_negative_numbers = true)]
        credits: String,
        /// Why: shown to the account's members with the entry.
        #[arg(long)]
        note: String,
    },
    /// Change an account's limits.
    Set {
        account: String,
        /// Never block this account for running out of credits.
        #[arg(long, value_enum)]
        unlimited: Option<OnOff>,
        /// Free credits each month, or `default`.
        #[arg(long)]
        monthly_grant: Option<String>,
        /// Calls a minute, or `default`.
        #[arg(long)]
        rate: Option<String>,
        /// Shown to the account's members with the entry.
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum OnOff {
    On,
    Off,
}

pub async fn run(config: Option<PathBuf>, cmd: CreditsCmd) -> Result<()> {
    let credits_cfg = CreditsConfig::from_env()?;
    let db = connect(config).await?;
    let (account, account_id) = match &cmd {
        CreditsCmd::Show { account, .. }
        | CreditsCmd::Grant { account, .. }
        | CreditsCmd::Adjust { account, .. }
        | CreditsCmd::Set { account, .. } => (account.clone(), resolve(&db, account).await?),
    };
    let not_found = || format!("no account matches {account:?}");
    match cmd {
        CreditsCmd::Show { json, .. } => {
            let view = credits_service::load_view(&db, &credits_cfg, account_id, true).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                print_view(&account, &view, true);
            }
            return Ok(());
        }
        CreditsCmd::Grant { credits, note, .. } => {
            let amount = parse_credits(&credits)?;
            credits_service::add_entry(&db, account_id, "grant", amount, note, &Actor::Operator)
                .await
                .map_err(|e| explain(e, not_found))?;
        }
        CreditsCmd::Adjust { credits, note, .. } => {
            let amount = parse_credits(&credits)?;
            credits_service::add_entry(
                &db,
                account_id,
                "adjustment",
                amount,
                Some(note),
                &Actor::Operator,
            )
            .await
            .map_err(|e| explain(e, not_found))?;
        }
        CreditsCmd::Set {
            unlimited,
            monthly_grant,
            rate,
            note,
            ..
        } => {
            if unlimited.is_none() && monthly_grant.is_none() && rate.is_none() {
                bail!("nothing to change: pass --unlimited, --monthly-grant or --rate");
            }
            let change = SettingsChange {
                monthly_free_grant_mc: monthly_grant
                    .as_deref()
                    .map(|v| or_default(v, parse_credits))
                    .transpose()?,
                rate_limit_per_min: rate
                    .as_deref()
                    .map(|v| {
                        or_default(v, |s| {
                            s.trim()
                                .parse::<i32>()
                                .with_context(|| format!("{s:?} isn't a number of calls a minute"))
                        })
                    })
                    .transpose()?,
                unlimited: unlimited.map(|u| matches!(u, OnOff::On)),
                note,
            };
            credits_service::update_settings(&db, account_id, change, &Actor::Operator)
                .await
                .map_err(|e| explain(e, not_found))?;
        }
    }
    let view = credits_service::load_view(&db, &credits_cfg, account_id, true).await?;
    print_view(&account, &view, false);
    Ok(())
}

async fn resolve(db: &PgPool, account: &str) -> Result<Uuid> {
    accounts::resolve_account(db, account).await.map_err(|e| {
        explain(e, || {
            format!(
                "no account matches {account:?}: use an account slug, an account id or a \
                 user's email (`chakramcp-server users list` shows each user's account)"
            )
        })
    })
}

/// `default` means back to the default (`None`); anything else is parsed.
fn or_default<T>(value: &str, parse: impl Fn(&str) -> Result<T>) -> Result<Option<T>> {
    if value.trim().eq_ignore_ascii_case("default") {
        Ok(None)
    } else {
        parse(value).map(Some)
    }
}

fn print_view(account: &str, view: &CreditsView, with_ledger: bool) {
    let default_note = |is_default: bool| if is_default { " (default)" } else { "" };
    println!("Account   {account} ({})", view.account_id);
    println!("Status    {}", view.status);
    println!("Balance   {} credits", format_credits(view.balance_mc));
    // A wallet created by a grant or a settings change hasn't had its first
    // monthly grant yet: the running server's credit worker adds it.
    let when = if view.has_wallet && view.last_grant_period.is_none() {
        "the first is added by the running server within seconds".to_owned()
    } else {
        format!("next on {}", view.next_grant_on)
    };
    println!(
        "Grant     {} credits a month{}, {when}",
        format_credits(view.monthly_free_grant_mc),
        default_note(view.monthly_free_grant_override_mc.is_none()),
    );
    println!(
        "Cost      {} credits a call",
        format_credits(view.cost_per_invocation_mc)
    );
    println!(
        "Rate      {} calls a minute{}",
        view.rate_limit_per_min,
        default_note(view.rate_limit_override_per_min.is_none()),
    );
    println!(
        "Spent     {} credits this month, {} calls",
        format_credits(view.spent_this_month_mc),
        view.invocations_this_month,
    );
    if !with_ledger || view.ledger.is_empty() {
        return;
    }
    println!();
    let rows: Vec<Vec<String>> = view
        .ledger
        .iter()
        .take(10)
        .map(|e| {
            let delta = if e.delta_mc > 0 {
                format!("+{}", format_credits(e.delta_mc))
            } else {
                format_credits(e.delta_mc)
            };
            vec![
                e.created_at.format("%Y-%m-%d %H:%M").to_string(),
                e.kind.clone(),
                delta,
                format_credits(e.balance_after_mc),
                e.by.clone().unwrap_or_default(),
                e.note.clone().unwrap_or_default(),
            ]
        })
        .collect();
    print_table(
        &["WHEN (UTC)", "KIND", "CHANGE", "BALANCE", "BY", "NOTE"],
        &rows,
    );
}
