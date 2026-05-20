//! `chakramcp org …` — view/manage organization accounts the caller belongs to.
//!
//! Currently surfaces:
//!   * `org list`           — every org the active user is a member of.
//!   * `org show <slug>`    — full org metadata including settings flags.
//!   * `org settings get`   — just the two settings fields (cheaper than `show`).
//!   * `org settings set`   — owner|admin only; flip default visibility,
//!     auto-friendship, or both.
//!
//! Mirrors the panel at `/app/orgs/{slug}/settings`. Server-side
//! validation (visibility ∈ {private,org,network}, role gate) is enforced
//! by the relay handler, so the CLI just passes the bytes through.

use anyhow::{anyhow, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List orgs (and personal accounts) the active user belongs to.
    List,
    /// Show a single org's metadata + role + settings.
    Show {
        /// Org slug, e.g. `acme-labs`.
        slug: String,
    },
    /// Get/set the per-org settings (default visibility, auto-friendship).
    #[command(subcommand)]
    Settings(SettingsCmd),
}

#[derive(Subcommand, Debug)]
pub enum SettingsCmd {
    /// Print just the settings fields for an org.
    Get {
        /// Org slug.
        slug: String,
    },
    /// Update one or both settings. Owner|admin only (server-enforced).
    /// At least one of --default-visibility, --auto-friendship, or
    /// --no-auto-friendship must be supplied.
    Set {
        slug: String,
        /// `private` | `org` | `network`.
        #[arg(long, value_name = "TIER")]
        default_visibility: Option<String>,
        /// Turn auto-friendship ON. Mutually exclusive with
        /// `--no-auto-friendship`.
        #[arg(long, conflicts_with = "no_auto_friendship")]
        auto_friendship: bool,
        /// Turn auto-friendship OFF. Mutually exclusive with
        /// `--auto-friendship`.
        #[arg(long)]
        no_auto_friendship: bool,
    },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List => {
            let v: Value = api.get_app("/v1/orgs").await?;
            print(&v)
        }
        Cmd::Show { slug } => {
            let v: Value = api.get_app(&format!("/v1/orgs/{}", slug)).await?;
            print(&v)
        }
        Cmd::Settings(SettingsCmd::Get { slug }) => {
            let v: Value = api.get_app(&format!("/v1/orgs/{}/settings", slug)).await?;
            print(&v)
        }
        Cmd::Settings(SettingsCmd::Set {
            slug,
            default_visibility,
            auto_friendship,
            no_auto_friendship,
        }) => {
            // Build a partial body that COALESCEs server-side. The
            // backend treats absent keys as "leave alone"; sending an
            // empty {} is a no-op.
            let mut body = json!({});
            if let Some(v) = default_visibility.as_deref() {
                if !matches!(v, "private" | "org" | "network") {
                    return Err(anyhow!("--default-visibility must be private|org|network"));
                }
                body["default_agent_visibility"] = json!(v);
            }
            if auto_friendship {
                body["auto_friendship_enabled"] = json!(true);
            } else if no_auto_friendship {
                body["auto_friendship_enabled"] = json!(false);
            }
            if body.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                return Err(anyhow!(
                    "nothing to change: pass --default-visibility and/or --(no-)auto-friendship"
                ));
            }
            let v: Value = api
                .put_app(&format!("/v1/orgs/{}/settings", slug), &body)
                .await?;
            print(&v)
        }
    }
}
