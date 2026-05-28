//! `chakramcp reviews …` — agent-to-agent ratings & reviews
//! (sub-project 2 of the ratings feature, backend in migration 0023).
//!
//! Five verbs:
//!   * `list <target>`              — paginated reviews + summary
//!   * `write <target> --from <…>`  — write/upsert a review
//!   * `eligibility <target>`       — which of your agents can review
//!   * `hide <target> <review_id>`  — target-owner moderation
//!   * `unhide <target> <review_id>`
//!
//! `target` accepts either a UUID or `account_slug/agent_slug` for
//! ergonomics; the latter resolves via `/v1/network/agents` plus a
//! filter pass. UUIDs hit the endpoint directly.

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::client::ApiClient;
use crate::print;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List reviews on a target agent. Returns the cursor-paginated
    /// list plus a summary (average + per-bucket counts). Owners can
    /// pass `--include-hidden` to see hidden rows; for non-owners the
    /// flag is silently ignored server-side.
    List {
        /// Target agent — UUID or `account_slug/agent_slug`.
        target: String,
        /// Filter by review tier.
        #[arg(long, value_parser = ["friend", "public"])]
        tier: Option<String>,
        /// Show reviews you've hidden (target-account members only).
        #[arg(long, default_value_t = false)]
        include_hidden: bool,
        /// Page-size hint to the relay (1-100; default 20).
        #[arg(long)]
        limit: Option<u32>,
        /// Pagination cursor from a prior `next_cursor`.
        #[arg(long)]
        cursor: Option<String>,
    },

    /// Write (or upsert) a review for a target agent.
    ///
    /// You must supply one of your agents via `--from` AND tag at
    /// least one capability you've actually invoked
    /// (`relay_invocations.status != 'rejected'`).
    Write {
        /// Target agent — UUID or `account_slug/agent_slug`.
        target: String,
        /// Reviewer agent (your side) — UUID or `account_slug/agent_slug`.
        #[arg(long)]
        from: String,
        /// 1–5 stars.
        #[arg(long)]
        rating: u8,
        /// Optional review text (up to 4 000 characters).
        #[arg(long)]
        comment: Option<String>,
        /// Capability IDs to tag. Repeat the flag to tag multiple.
        /// At least one is required by the backend.
        #[arg(long = "tag", value_name = "CAPABILITY_ID", required = true)]
        tags: Vec<String>,
    },

    /// Show which of your agents can leave a review for the target,
    /// and which of the target's capabilities each has invoked. Use
    /// this to decide which `--from` + `--tag` to pass to `write`.
    Eligibility {
        /// Target agent — UUID or `account_slug/agent_slug`.
        target: String,
    },

    /// Hide a review on one of your agents. Only target-account
    /// members can hide; the reviewer can't hide their own row.
    Hide {
        /// Target agent — UUID or `account_slug/agent_slug`.
        target: String,
        /// Review UUID to hide.
        review_id: String,
    },

    /// Unhide a previously hidden review.
    Unhide {
        /// Target agent — UUID or `account_slug/agent_slug`.
        target: String,
        /// Review UUID to unhide.
        review_id: String,
    },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::List {
            target,
            tier,
            include_hidden,
            limit,
            cursor,
        } => {
            let target_id = resolve_target(&api, &target).await?;
            let mut path = format!("/v1/agents/{target_id}/reviews");
            let mut qs: Vec<String> = Vec::new();
            if let Some(t) = tier {
                qs.push(format!("tier={t}"));
            }
            if include_hidden {
                qs.push("include_hidden=true".into());
            }
            if let Some(n) = limit {
                qs.push(format!("limit={n}"));
            }
            if let Some(c) = cursor {
                qs.push(format!("cursor={}", urlencode(&c)));
            }
            if !qs.is_empty() {
                path.push('?');
                path.push_str(&qs.join("&"));
            }
            let v: Value = api.get_relay(&path).await?;
            print(&v)
        }
        Cmd::Write {
            target,
            from,
            rating,
            comment,
            tags,
        } => {
            if !(1..=5).contains(&rating) {
                anyhow::bail!("rating must be between 1 and 5 (got {rating})");
            }
            let target_id = resolve_target(&api, &target).await?;
            let reviewer_id = resolve_target(&api, &from).await?;
            let body = json!({
                "reviewer_agent_id": reviewer_id,
                "rating": rating,
                "comment": comment,
                "tagged_capability_ids": tags,
            });
            let v: Value = api
                .post_relay(&format!("/v1/agents/{target_id}/reviews"), &body)
                .await?;
            print(&v)
        }
        Cmd::Eligibility { target } => {
            let target_id = resolve_target(&api, &target).await?;
            let v: Value = api
                .get_relay(&format!("/v1/agents/{target_id}/reviews/eligibility"))
                .await?;
            print(&v)
        }
        Cmd::Hide { target, review_id } => {
            let target_id = resolve_target(&api, &target).await?;
            let v: Value = api
                .post_relay(
                    &format!("/v1/agents/{target_id}/reviews/{review_id}/hide"),
                    &json!({}),
                )
                .await?;
            print(&v)
        }
        Cmd::Unhide { target, review_id } => {
            let target_id = resolve_target(&api, &target).await?;
            let v: Value = api
                .post_relay(
                    &format!("/v1/agents/{target_id}/reviews/{review_id}/unhide"),
                    &json!({}),
                )
                .await?;
            print(&v)
        }
    }
}

/// Accept either a UUID or `account_slug/agent_slug`. The slug form is
/// resolved by scanning `/v1/network/agents` (visibility=network or
/// org-shared) for a match. We do a single page-200 fetch which is
/// fine for tens-of-thousands-of-agents scale; if it ever grows past
/// that, swap in `/v1/discovery/agents` which is search-indexed.
async fn resolve_target(api: &ApiClient, ident: &str) -> Result<Uuid> {
    if let Ok(id) = Uuid::parse_str(ident) {
        return Ok(id);
    }
    let (account_slug, agent_slug) = ident.split_once('/').with_context(|| {
        format!("agent identifier must be UUID or `account_slug/agent_slug` (got `{ident}`)")
    })?;
    // Page through up to 200 results. That's wider than the directory
    // page size to give slug-resolution headroom; we still need only
    // one round-trip per command.
    let body: Value = api.get_relay("/v1/network/agents?limit=100").await?;
    let agents = body
        .get("agents")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("unexpected /v1/network/agents shape"))?;
    for a in agents {
        let a_acc = a.get("account_slug").and_then(|v| v.as_str());
        let a_slug = a.get("slug").and_then(|v| v.as_str());
        if a_acc == Some(account_slug) && a_slug == Some(agent_slug) {
            let id = a
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("agent row missing id"))?;
            return Uuid::parse_str(id).context("malformed agent id from relay");
        }
    }
    anyhow::bail!(
        "no agent matched `{account_slug}/{agent_slug}` on the active network — \
         try a UUID, or run `chakramcp network` to list visible agents."
    )
}

fn urlencode(s: &str) -> String {
    // Cursor values come from the relay; they're base64url, so no
    // exotic chars. A tiny safe-subset escaper avoids pulling in
    // percent-encoding for a few characters.
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}
