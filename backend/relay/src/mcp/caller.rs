//! Who a call by name is made as (MCP and discovery spec §6.1), and the
//! agents the relay makes for callers who have none to call as (§6.4).
//!
//! Per capability: a named agent of yours (`as`), else one of yours holding
//! a grant for it (the oldest grant), else — for a public capability — your
//! default agent, else `chk.policy.grant_required`.

use axum::http::StatusCode;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use crate::auth::{AgentScopeMode, AuthUser};
use crate::discovery::fields::Access;
use crate::discovery::profile::{self, AgentRef};
use crate::discovery::standing::{self, Mine};
use crate::handlers::invoke::Target;
use crate::state::RelayState;

/// A call by name, resolved: the grant or public capability to invoke, and
/// the agent to call as.
#[derive(Debug)]
pub(crate) struct Resolved {
    pub(crate) target: Target,
    pub as_agent: Uuid,
}

/// Resolve a call to `capability` on `agent` for `user`'s credential.
pub(crate) async fn resolve(
    state: &RelayState,
    user: &AuthUser,
    agent: &AgentRef,
    capability: &str,
    as_agent: Option<Uuid>,
) -> Result<Resolved, ApiError> {
    let name = describe(agent);
    let target = profile::resolve(state, Some(user.user_id), agent)
        .await?
        .ok_or_else(|| {
            ApiError::coded(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("no agent {name} you can see"),
            )
        })?;
    let cap = sqlx::query!(
        r#"SELECT id, public_invoke, visibility FROM agent_capabilities
            WHERE agent_id = $1 AND name = $2"#,
        target.summary.id,
        capability,
    )
    .fetch_optional(&state.db)
    .await?
    .filter(|c| target.relation.sees(&c.visibility))
    .ok_or_else(|| {
        ApiError::coded(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{name} has no capability `{capability}` you can see"),
        )
    })?;
    let public = Access::of(cap.public_invoke, &target.summary.visibility) == Access::Public;
    let mine = standing::my_agents(state, user).await?;

    let candidates: Vec<Uuid> = match as_agent {
        Some(id) => {
            if !mine.iter().any(|a| a.id == id) {
                return Err(not_yours(state, user, id).await?);
            }
            vec![id]
        }
        None => mine.iter().map(|a| a.id).collect(),
    };
    let grant = sqlx::query!(
        r#"
        SELECT id, grantee_agent_id FROM grants
         WHERE granter_agent_id = $1 AND capability_id = $2
           AND grantee_agent_id = ANY($3)
           AND status = 'active' AND (expires_at IS NULL OR expires_at > now())
         ORDER BY granted_at, id
         LIMIT 1
        "#,
        target.summary.id,
        cap.id,
        &candidates,
    )
    .fetch_optional(&state.db)
    .await?;
    if let Some(g) = grant {
        return Ok(Resolved {
            target: Target::Grant(g.id),
            as_agent: g.grantee_agent_id,
        });
    }
    if !public {
        return Err(ApiError::Policy {
            code: "chk.policy.grant_required",
            message: format!(
                "`{capability}` on {name} needs a grant. Propose a friendship with {name} from \
                 one of your agents (propose_friendship), and once it's accepted ask its owner \
                 to grant `{capability}`."
            ),
        });
    }
    let as_agent = match as_agent {
        Some(id) => id,
        None => default_for(state, user, &mine).await?,
    };
    Ok(Resolved {
        target: Target::Public(cap.id),
        as_agent,
    })
}

/// How `agent` was named, for messages.
fn describe(agent: &AgentRef) -> String {
    match agent {
        AgentRef::Id(id) => id.to_string(),
        AgentRef::Name { account, slug } => format!("{account}/{slug}"),
    }
}

/// The refusal for calling as an agent outside the caller's reach: out of
/// the credential's scope if it's in one of the user's accounts, else not
/// theirs.
async fn not_yours(state: &RelayState, user: &AuthUser, agent: Uuid) -> Result<ApiError, ApiError> {
    let member = sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM agents a
                 JOIN account_memberships m ON m.account_id = a.account_id AND m.user_id = $2
                WHERE a.id = $1 AND a.tombstoned_at IS NULL
           ) AS "member!""#,
        agent,
        user.user_id,
    )
    .fetch_one(&state.db)
    .await?;
    Ok(if member {
        crate::auth::out_of_scope()
    } else {
        ApiError::coded(
            StatusCode::FORBIDDEN,
            "forbidden",
            "you can only call as one of your own agents",
        )
    })
}

fn as_required() -> ApiError {
    ApiError::coded(
        StatusCode::BAD_REQUEST,
        "chk.mcp.as_required",
        "name the agent to call as with ?as= (or grantee_agent_id)",
    )
}

/// The agent a public call is made as when the caller names none (§6.1 rule
/// 3): an unscoped credential's personal agent; a scoped credential's one
/// agent in scope; an `own` credential's client agent when it has none.
async fn default_for(state: &RelayState, user: &AuthUser, mine: &[Mine]) -> Result<Uuid, ApiError> {
    let grant = crate::auth::resolve_grant(&state.db, user).await?;
    match grant.mode {
        AgentScopeMode::All => default_agent(state, user, Kind::Personal).await,
        _ if mine.len() == 1 => Ok(mine[0].id),
        AgentScopeMode::Own if mine.is_empty() => {
            let kind = match (user.api_key_id, grant.client_id) {
                (Some(key), _) => Kind::ApiKey(key),
                (None, Some(client)) => Kind::OAuth(client),
                (None, None) => return Err(as_required()),
            };
            default_agent(state, user, kind).await
        }
        _ => Err(as_required()),
    }
}

/// Which default agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// An unscoped credential's: "<user> (personal)".
    Personal,
    /// An `own`-scoped OAuth or device credential's: "<user> via <client>".
    OAuth(String),
    /// An `own`-scoped API key's: "<user> via <key name>".
    ApiKey(Uuid),
}

impl Kind {
    fn key(&self, user: Uuid) -> String {
        match self {
            Kind::Personal => format!("user:{user}"),
            Kind::OAuth(client) => format!("oauth:{client}:{user}"),
            Kind::ApiKey(id) => format!("api_key:{id}"),
        }
    }
}

/// The default agent of `kind` for `user`, made the first time it's needed
/// (§6.4). Concurrent first calls make one agent: creation holds an
/// advisory lock on the key. A deleted default agent is replaced.
pub async fn default_agent(
    state: &RelayState,
    user: &AuthUser,
    kind: Kind,
) -> Result<Uuid, ApiError> {
    let key = kind.key(user.user_id);
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(&key)
        .execute(&mut *tx)
        .await?;
    let existing = sqlx::query_scalar!(
        r#"SELECT d.agent_id FROM default_agents d
             JOIN agents a ON a.id = d.agent_id
            WHERE d.key = $1 AND a.tombstoned_at IS NULL"#,
        key,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(agent) = existing {
        return Ok(agent);
    }

    let home = sqlx::query!(
        r#"SELECT a.id, u.display_name AS user_name
             FROM accounts a
             JOIN users u ON u.id = a.owner_user_id
            WHERE a.owner_user_id = $1 AND a.account_type = 'individual'
              AND a.tombstoned_at IS NULL
            ORDER BY a.created_at
            LIMIT 1"#,
        user.user_id,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        ApiError::coded(
            StatusCode::CONFLICT,
            "chk.mcp.as_required",
            "you have no personal account for the relay to make an agent in; \
             name the agent to call as with ?as= (or grantee_agent_id)",
        )
    })?;

    let (display_name, slug_base, description, client_id, api_key_id) = match &kind {
        Kind::Personal => (
            format!("{} (personal)", home.user_name),
            "personal".to_owned(),
            "Made by ChakraMCP for calls you make from an MCP client without naming one of \
             your agents. You can rename it, give it capabilities or delete it."
                .to_owned(),
            None,
            None,
        ),
        Kind::OAuth(client) => {
            let name = sqlx::query_scalar!(
                "SELECT client_name FROM oauth_clients WHERE client_id = $1",
                client,
            )
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_else(|| client.clone());
            client_agent(&home.user_name, &name, Some(client.clone()), None)
        }
        Kind::ApiKey(id) => {
            let name = sqlx::query_scalar!("SELECT name FROM api_keys WHERE id = $1", id)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or_else(|| "an API key".to_owned());
            client_agent(&home.user_name, &name, None, Some(*id))
        }
    };

    let id = Uuid::now_v7();
    let mut slug = slug_base.clone();
    for attempt in 0..6 {
        let inserted = sqlx::query_scalar!(
            r#"
            INSERT INTO agents
                (id, account_id, created_by_user_id, slug, display_name, description,
                 visibility, mode, accept_calls, created_by_client_id, created_by_api_key_id)
            VALUES ($1, $2, $3, $4, $5, $6, 'private', 'pull', 'nobody', $7, $8)
            ON CONFLICT (account_id, slug) WHERE tombstoned_at IS NULL DO NOTHING
            RETURNING id
            "#,
            id,
            home.id,
            user.user_id,
            slug,
            display_name,
            description,
            client_id,
            api_key_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if inserted.is_some() {
            break;
        }
        if attempt == 5 {
            return Err(ApiError::Conflict(format!(
                "couldn't find a free slug for {display_name}"
            )));
        }
        slug = format!("{slug_base}-{}", &Uuid::now_v7().simple().to_string()[28..]);
    }
    sqlx::query!(
        r#"INSERT INTO default_agents (key, user_id, agent_id) VALUES ($1, $2, $3)
           ON CONFLICT (key) DO UPDATE SET agent_id = EXCLUDED.agent_id, created_at = now()"#,
        key,
        user.user_id,
        id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // An `own` credential's new agent is in its scope from now on.
    state.scope_cache.forget(crate::auth::credential(user));
    crate::events::record_audit(
        &state.db,
        user,
        Some(home.id),
        "agent.create",
        "agent",
        Some(id),
        None,
        &format!("made default agent {slug}"),
        serde_json::json!({ "slug": slug, "default": key.split(':').next() }),
    )
    .await;
    Ok(id)
}

/// A client agent's name, slug base and description.
fn client_agent(
    user_name: &str,
    client: &str,
    client_id: Option<String>,
    api_key_id: Option<Uuid>,
) -> (String, String, String, Option<String>, Option<Uuid>) {
    (
        format!("{user_name} via {client}"),
        client_slug(client),
        format!(
            "Made by ChakraMCP for {client} to call agents as, since it had no agent of its own. \
             You can rename it, give it capabilities or delete it."
        ),
        client_id,
        api_key_id,
    )
}

/// `via-<client>`, slug-shaped and short enough for a `-xxxx` suffix.
fn client_slug(client: &str) -> String {
    let mut slug = String::from("via-");
    let mut dash = true;
    for c in client.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            dash = false;
        } else if !dash {
            slug.push('-');
            dash = true;
        }
        if slug.len() >= 27 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    if slug == "via" {
        "via-client".to_owned()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_slugs_are_slug_shaped() {
        assert_eq!(client_slug("Claude Desktop"), "via-claude-desktop");
        assert_eq!(client_slug("  ✨ "), "via-client");
        assert_eq!(client_slug("A--B__c"), "via-a-b-c");
        let long = client_slug("an extremely long client name that goes on");
        assert!(long.len() <= 27 && !long.ends_with('-'), "{long}");
    }
}
