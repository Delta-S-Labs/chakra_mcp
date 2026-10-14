//! Operator commands: `chakramcp-server users …` and `chakramcp-server
//! credits …`. They talk to the database directly and need no sign-in, so
//! they're for whoever runs the server: inside its container
//! (`docker compose exec relay chakramcp-server …`, `kubectl exec`) or on
//! the host. Results go to stdout and diagnostics to stderr, so `--json`
//! output stays parseable.

pub mod credits;
pub mod users;

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing_subscriber::EnvFilter;

use chakramcp_shared::error::ApiError;

/// Load the server config the way `migrate` does, quietly, and connect.
async fn connect(explicit_path: Option<PathBuf>) -> Result<(crate::ServerConfig, PgPool)> {
    // Warnings only, on stderr: info lines would mix with the results.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::new("warn"))
        .try_init();
    let cfg = crate::load_config(explicit_path)?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&cfg.shared.database_url)
        .await
        .with_context(|| {
            format!(
                "connecting to {}",
                crate::redact_url(&cfg.shared.database_url)
            )
        })?;
    Ok((cfg, pool))
}

/// An API error as a message for the person at the terminal. `not_found`
/// names what was missing.
fn explain(err: ApiError, not_found: impl FnOnce() -> String) -> anyhow::Error {
    match err {
        ApiError::NotFound => anyhow!(not_found()),
        ApiError::InvalidRequest(message) | ApiError::Conflict(message) => anyhow!(message),
        other => anyhow::Error::new(other),
    }
}

/// A password from stdin (`--password-stdin`, one line) or a prompt on the
/// terminal, asked twice.
fn read_password(from_stdin: bool) -> Result<String> {
    if from_stdin {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("reading the password from stdin")?;
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        return Ok(line.to_owned());
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "no terminal to ask for a password on: pipe it in with --password-stdin \
             (with Compose, `exec -T`; with kubectl, `exec -i`)"
        );
    }
    dialoguer::Password::new()
        .with_prompt("Password")
        .with_confirmation("Repeat the password", "The passwords don't match")
        .interact()
        .context("reading the password")
}

/// Credits as typed (`5`, `0.5`, `-1.25`) to milli-credits. Three decimals
/// at most: 0.001 credit is the smallest unit.
fn parse_credits(input: &str) -> Result<i64> {
    let s = input.trim();
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let all_digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    if (whole.is_empty() && fraction.is_empty()) || !all_digits(whole) || !all_digits(fraction) {
        bail!("{input:?} isn't an amount of credits (e.g. 5, 0.5 or -1.25)");
    }
    if fraction.len() > 3 {
        bail!("{input:?} has more than three decimals: 0.001 credit is the smallest unit");
    }
    let too_large = || anyhow!("{input:?} is too large");
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| too_large())?
    };
    let fraction_mc: i64 = format!("{fraction:0<3}").parse().unwrap_or(0);
    let mc = whole
        .checked_mul(1_000)
        .and_then(|w| w.checked_add(fraction_mc))
        .ok_or_else(too_large)?;
    Ok(if negative { -mc } else { mc })
}

/// Milli-credits as credits: `99700` → `99.7`.
fn format_credits(mc: i64) -> String {
    let sign = if mc < 0 { "-" } else { "" };
    let abs = mc.unsigned_abs();
    let (whole, fraction) = (abs / 1_000, abs % 1_000);
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = format!("{fraction:03}");
        format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
    }
}

/// Left-aligned columns, two spaces apart.
fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_parse_to_milli_credits() {
        for (input, mc) in [
            ("5", 5_000),
            ("0.5", 500),
            (".5", 500),
            ("1.", 1_000),
            ("-1.25", -1_250),
            ("+2", 2_000),
            (" 0.001 ", 1),
            ("0", 0),
        ] {
            assert_eq!(parse_credits(input).unwrap(), mc, "{input}");
        }
        for bad in [
            "",
            "-",
            ".",
            "abc",
            "1.2345",
            "1.2.3",
            "1e3",
            "--1",
            "9223372036854775807",
        ] {
            assert!(parse_credits(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn milli_credits_format_as_credits() {
        for (mc, text) in [
            (0, "0"),
            (5_000, "5"),
            (99_700, "99.7"),
            (1, "0.001"),
            (-50, "-0.05"),
            (-1_250, "-1.25"),
        ] {
            assert_eq!(format_credits(mc), text, "{mc}");
        }
    }
}
