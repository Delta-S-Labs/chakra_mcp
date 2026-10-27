//! Agent profiles (spec §5.3): one agent by id or `account/agent`, with its
//! owner, reviews, capabilities in full and how to call it.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::fields::{self, Access, AgentSummary, Loaded};
use super::standing::{self, You};
use crate::auth::AuthUser;
use crate::state::RelayState;

/// How many reviews a profile shows.
const LATEST_REVIEWS: i64 = 3;

/// A way of naming an agent: its id, or `account_slug/agent_slug`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRef {
    Id(Uuid),
    Name { account: String, slug: String },
}

impl AgentRef {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Ok(id) = Uuid::parse_str(s) {
            return Some(Self::Id(id));
        }
        let (account, slug) = s.split_once('/')?;
        (!account.is_empty() && !slug.is_empty() && !slug.contains('/')).then(|| Self::Name {
            account: account.to_lowercase(),
            slug: slug.to_lowercase(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    #[serde(flatten)]
    pub agent: AgentSummary,
    pub owner: Owner,
    pub reviews: Reviews,
    pub capabilities: Vec<CapabilityDetail>,
    pub how_to_call: HowToCall,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub you: Option<You>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Owner {
    pub account_slug: String,
    pub display_name: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reviews {
    pub avg_rating: Option<f64>,
    pub count: i64,
    pub latest: Vec<Review>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub rating: i16,
    pub text: Option<String>,
    pub at: DateTime<Utc>,
    /// The reviewer agent's display name when that agent is
    /// network-visible, else "An agent".
    pub by: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityDetail {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub access: Access,
    /// Calls per calling agent per month, for a public capability.
    pub public_monthly_quota_per_agent: Option<i32>,
    /// Asks its owner before running.
    pub consent_required: bool,
    pub semantics: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HowToCall {
    pub cli: String,
    /// The agent's A2A card (network-visible agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a2a_card: Option<String>,
    /// Its page on the web (network-visible agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// The live agent `r` names, loaded for `viewer`, if the viewer may see it.
/// A renamed agent's old slug still finds it while its alias lasts.
pub async fn resolve(
    state: &RelayState,
    viewer: Option<Uuid>,
    r: &AgentRef,
) -> Result<Option<Loaded>, ApiError> {
    let id = match r {
        AgentRef::Id(id) => Some(*id),
        AgentRef::Name { account, slug } => by_name(state, account, slug).await?,
    };
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(fields::load(state, viewer, &[id])
        .await?
        .into_iter()
        .next()
        .filter(|l| l.relation.sees(&l.summary.visibility)))
}

async fn by_name(state: &RelayState, account: &str, slug: &str) -> Result<Option<Uuid>, ApiError> {
    let mut slug = slug.to_owned();
    // A rename chain collapses to its head; bounded against cycles.
    for _ in 0..8 {
        let row = sqlx::query!(
            r#"
            SELECT acc.id AS account_id,
                   (SELECT a.id FROM agents a
                     WHERE a.account_id = acc.id AND a.slug = $2 AND a.tombstoned_at IS NULL) AS agent_id,
                   (SELECT sa.new_slug FROM slug_aliases sa
                     WHERE sa.scope = 'agent' AND sa.account_id = acc.id
                       AND sa.old_slug = $2 AND sa.expires_at > now()) AS renamed_to
              FROM accounts acc
             WHERE acc.slug = $1 AND acc.tombstoned_at IS NULL
            "#,
            account,
            slug,
        )
        .fetch_optional(&state.db)
        .await?;
        match row {
            None => return Ok(None),
            Some(r) if r.agent_id.is_some() => return Ok(r.agent_id),
            Some(r) => match r.renamed_to {
                Some(next) if next != slug => slug = next,
                _ => return Ok(None),
            },
        }
    }
    Ok(None)
}

/// An agent's newest un-hidden reviews, up to `limit`. A reviewer agent is
/// named only when it's live and network-visible; otherwise "An agent".
pub async fn reviews(
    state: &RelayState,
    agent_id: Uuid,
    limit: i64,
) -> Result<Vec<Review>, ApiError> {
    Ok(sqlx::query!(
        r#"
        SELECT r.rating, r.comment, r.created_at,
               CASE WHEN ra.visibility = 'network' AND ra.tombstoned_at IS NULL
                    THEN ra.display_name END AS reviewer
          FROM agent_reviews r
          JOIN agents ra ON ra.id = r.reviewer_agent_id
         WHERE r.target_agent_id = $1 AND r.hidden_at IS NULL
         ORDER BY r.created_at DESC
         LIMIT $2
        "#,
        agent_id,
        limit,
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|r| Review {
        rating: r.rating,
        text: r.comment,
        at: r.created_at,
        by: r.reviewer.unwrap_or_else(|| "An agent".to_owned()),
    })
    .collect())
}

/// `loaded`'s profile for `viewer`. Two queries, and standing's when signed
/// in.
pub async fn profile(
    state: &RelayState,
    viewer: Option<&AuthUser>,
    loaded: Loaded,
) -> Result<Profile, ApiError> {
    let agent = &loaded.summary;
    let capabilities: Vec<CapabilityDetail> = sqlx::query!(
        r#"
        SELECT id, name, description, input_schema, output_schema, visibility, public_invoke,
               public_monthly_quota_per_agent, consent_required, semantics
          FROM agent_capabilities
         WHERE agent_id = $1
         ORDER BY name
        "#,
        agent.id,
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .filter(|c| loaded.relation.sees(&c.visibility))
    .map(|c| CapabilityDetail {
        id: c.id,
        name: c.name,
        description: c.description,
        input_schema: c.input_schema,
        output_schema: c.output_schema,
        access: Access::of(c.public_invoke, &agent.visibility),
        public_monthly_quota_per_agent: c
            .public_invoke
            .then_some(c.public_monthly_quota_per_agent)
            .flatten(),
        consent_required: c.consent_required,
        semantics: c.semantics,
    })
    .collect();

    let latest = reviews(state, agent.id, LATEST_REVIEWS).await?;

    let name = format!("{}/{}", agent.account_slug, agent.slug);
    let public = agent.visibility == "network";
    let how_to_call = HowToCall {
        cli: format!(
            "chakramcp invoke ensure {name} {} '{{}}' --from <your-agent> --wait",
            capabilities
                .first()
                .map_or("<capability>", |c| c.name.as_str()),
        ),
        a2a_card: public.then(|| {
            format!(
                "{}/agents/{name}/.well-known/agent-card.json",
                state.config.relay_base_url.trim_end_matches('/')
            )
        }),
        profile: public.then(|| {
            format!(
                "{}/agents/{name}",
                state.config.frontend_base_url.trim_end_matches('/')
            )
        }),
    };

    let you = match viewer {
        Some(user) => {
            let mine = standing::my_agents(state, user).await?;
            standing::standing(state, &mine, std::slice::from_ref(&loaded))
                .await?
                .remove(&agent.id)
        }
        None => None,
    };

    Ok(Profile {
        owner: Owner {
            account_slug: agent.account_slug.clone(),
            display_name: loaded.account_display_name.clone(),
            verified: agent.verified,
        },
        reviews: Reviews {
            avg_rating: agent.avg_rating,
            count: agent.review_count,
            latest,
        },
        capabilities,
        how_to_call,
        you,
        agent: loaded.summary,
    })
}
