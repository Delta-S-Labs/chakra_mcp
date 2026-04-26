//! Stderr-side UI helpers: branded banner, colored status lines,
//! interactive prompts, spinner during long ops. Stdout stays
//! reserved for machine-readable JSON output of commands.

use std::time::Duration;

use anyhow::Result;
use dialoguer::{theme::ColorfulTheme, Input, Password, Select};
use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;

const BANNER_TOP: &str = "  ╭─────────────────────────────────────╮";
const BANNER_BOT: &str = "  ╰─────────────────────────────────────╯";

/// First-run welcome banner. Coral border, butter glyph.
pub fn banner() {
    eprintln!();
    eprintln!("{}", BANNER_TOP.bright_black());
    eprintln!(
        "  {}  {}  {}            {}",
        "│".bright_black(),
        "⌬".bright_yellow(),
        "chakramcp".bold(),
        "│".bright_black(),
    );
    eprintln!(
        "  {}     {}        {}",
        "│".bright_black(),
        "relay for agents".dimmed(),
        "│".bright_black(),
    );
    eprintln!("{}", BANNER_BOT.bright_black());
    eprintln!();
}

/// Compact status header: `==> what we're doing`.
pub fn step(msg: &str) {
    eprintln!("{} {}", "==>".bright_yellow().bold(), msg.bold());
}

/// Success line: green check.
pub fn ok(msg: &str) {
    eprintln!("{} {}", "✓".green().bold(), msg);
}

/// Soft note: dimmed prefix.
pub fn note(msg: &str) {
    eprintln!("{} {}", "·".bright_black(), msg.dimmed());
}

/// Error line — same style across the CLI; main() uses this on the way out.
pub fn err(msg: &str) {
    eprintln!("{} {}", "✗".red().bold(), msg);
}

/// Open spinner that updates in place. Returns a handle the caller
/// finishes when the work is done.
pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.enable_steady_tick(Duration::from_millis(80));
    pb.set_message(msg.to_string());
    pb
}

// ─── Prompts ─────────────────────────────────────────────

pub fn select(prompt: &str, items: &[&str], default: usize) -> Result<usize> {
    let theme = ColorfulTheme::default();
    Ok(Select::with_theme(&theme)
        .with_prompt(prompt)
        .items(items)
        .default(default)
        .interact()?)
}

pub fn input(prompt: &str, default: Option<&str>) -> Result<String> {
    let theme = ColorfulTheme::default();
    let mut p = Input::<String>::with_theme(&theme).with_prompt(prompt);
    if let Some(d) = default {
        p = p.default(d.to_string());
    }
    Ok(p.interact_text()?)
}

pub fn password(prompt: &str) -> Result<String> {
    let theme = ColorfulTheme::default();
    Ok(Password::with_theme(&theme).with_prompt(prompt).interact()?)
}

/// Render the "you're in" closing of the onboarding flow, with a
/// short list of "try this next" commands.
pub fn closing(network_name: &str, account: &str) {
    eprintln!();
    eprintln!(
        "{} signed in as {}",
        "✓".green().bold(),
        account.bold(),
    );
    eprintln!(
        "{} on the {} network",
        "✓".green().bold(),
        network_name.bold(),
    );
    eprintln!();
    eprintln!("  {}", "Try:".dimmed());
    let cmds: &[(&str, &str)] = &[
        ("chakramcp agents list", "your agents"),
        ("chakramcp network", "discover others on this network"),
        ("chakramcp grants list", "what you can call / who can call you"),
        ("chakramcp inbox pull --agent <id>", "claim pending work"),
    ];
    for (cmd, desc) in cmds {
        eprintln!("    {}   {}", cmd.bright_yellow(), desc.dimmed());
    }
    eprintln!();
}
