//! Discovery over REST (MCP and discovery spec §5): ranked search,
//! featured agents, profiles and the caller's standing. The queries live in
//! `crate::discovery`, shared with the network list and the MCP tools.
//!
//! - `GET /v1/discovery/agents`: search (`discovery::search`).
//! - `GET /v1/discovery/featured`: agents worth trying first.
//! - `GET /v1/discovery/agents/{account_slug}/{agent_slug}` and
//!   `GET /v1/discovery/agents/{id}`: a profile.
//! - `GET /v1/discovery/standing?agent=<ref>`: your standing with one agent.
//!
//! Signed out, these answer for the public directory and responses are
//! cacheable. With a token they add your agents, org agents you share and
//! your standing (`you`), and nothing is cached (`discovery::cache_headers`).
//! With `DISCOVERY_V2` off, search and featured are 404 and profiles and
//! standing need sign-in.

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::discovery::featured::{self, FeaturedAgent};
use crate::discovery::profile::{self, AgentRef};
use crate::discovery::search::{self, SearchParams};
use crate::discovery::{cache_headers, standing, MaybeUser};
use crate::state::RelayState;

/// Lists and profiles: a short browser cache, a longer edge cache.
const PUBLIC_CACHE: &str = "public, max-age=30, s-maxage=120";

/// `GET /v1/discovery/agents`
pub async fn search(
    State(state): State<RelayState>,
    MaybeUser(viewer): MaybeUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Response> {
    if !state.config.discovery_v2_enabled {
        return Err(ApiError::NotFound);
    }
    let page = search::search(&state, viewer.as_ref(), params).await?;
    Ok((cache_headers(viewer.is_some(), PUBLIC_CACHE), Json(page)).into_response())
}

// ─── Featured ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct FeaturedResponse {
    pub agents: Vec<FeaturedAgent>,
}

/// `GET /v1/discovery/featured`: agents worth trying first
/// (`discovery::featured`).
pub async fn featured(
    State(state): State<RelayState>,
    MaybeUser(viewer): MaybeUser,
) -> ApiResult<Response> {
    if !state.config.discovery_v2_enabled {
        return Err(ApiError::NotFound);
    }
    let agents = featured::featured(&state, viewer.as_ref().map(|u| u.user_id)).await?;
    Ok((
        cache_headers(viewer.is_some(), "public, max-age=30, s-maxage=60"),
        Json(FeaturedResponse { agents }),
    )
        .into_response())
}

// ─── Profiles and standing ─────────────────────────────────

/// `GET /v1/discovery/agents/{account_slug}/{agent_slug}`
pub async fn profile_by_name(
    State(state): State<RelayState>,
    MaybeUser(viewer): MaybeUser,
    Path((account_slug, agent_slug)): Path<(String, String)>,
) -> ApiResult<Response> {
    let r = AgentRef::Name {
        account: account_slug.to_lowercase(),
        slug: agent_slug.to_lowercase(),
    };
    profile_response(&state, viewer, r).await
}

/// `GET /v1/discovery/agents/{id}`
pub async fn profile_by_id(
    State(state): State<RelayState>,
    MaybeUser(viewer): MaybeUser,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let id = Uuid::parse_str(&id).map_err(|_| ApiError::NotFound)?;
    profile_response(&state, viewer, AgentRef::Id(id)).await
}

async fn profile_response(
    state: &RelayState,
    viewer: Option<AuthUser>,
    r: AgentRef,
) -> ApiResult<Response> {
    if !state.config.discovery_v2_enabled && viewer.is_none() {
        return Err(ApiError::Unauthorized);
    }
    let loaded = profile::resolve(state, viewer.as_ref().map(|u| u.user_id), &r)
        .await?
        .ok_or(ApiError::NotFound)?;
    let body = profile::profile(state, viewer.as_ref(), loaded).await?;
    Ok((cache_headers(viewer.is_some(), PUBLIC_CACHE), Json(body)).into_response())
}

#[derive(Debug, Deserialize)]
pub struct StandingQuery {
    /// The agent: its id or `account_slug/agent_slug`.
    pub agent: String,
}

/// `GET /v1/discovery/standing?agent=<ref>`: your standing with one agent.
pub async fn standing(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<StandingQuery>,
) -> ApiResult<Response> {
    let r = AgentRef::parse(&q.agent).ok_or_else(|| {
        ApiError::InvalidRequest("agent must be an id or account_slug/agent_slug".into())
    })?;
    let loaded = profile::resolve(&state, Some(user.user_id), &r)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mine = standing::my_agents(&state, &user).await?;
    let id = loaded.summary.id;
    let you = standing::standing(&state, &mine, &[loaded])
        .await?
        .remove(&id)
        .unwrap_or_default();
    Ok((cache_headers(true, PUBLIC_CACHE), Json(you)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: true,
            log_filter: "warn".into(),
        }
    }

    async fn seed_account(pool: &PgPool, slug: &str, verified: bool) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type)
               VALUES ($1, $2, 'Acme Corp', 'individual')"#,
            id,
            slug,
        )
        .execute(pool)
        .await
        .unwrap();
        if verified {
            sqlx::query!(
                "UPDATE accounts SET verified_at = now(), verification_method = 'dns_txt' WHERE id = $1",
                id,
            )
            .execute(pool)
            .await
            .unwrap();
        }
        id
    }

    /// Insert a network-visibility agent with optional capability + tags.
    async fn seed_agent(
        pool: &PgPool,
        account_id: Uuid,
        slug: &str,
        display_name: &str,
        description: &str,
        mode: &str,
        tags: &[&str],
    ) -> Uuid {
        let id = Uuid::now_v7();
        let card_url = if mode == "push" {
            Some("https://example.com/.well-known/agent-card.json".to_string())
        } else {
            None
        };
        let tags_owned: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        sqlx::query!(
            r#"INSERT INTO agents
                  (id, account_id, slug, display_name, description, visibility, mode, agent_card_url, tags)
               VALUES ($1, $2, $3, $4, $5, 'network', $6, $7, $8)"#,
            id,
            account_id,
            slug,
            display_name,
            description,
            mode,
            card_url,
            &tags_owned,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn add_capability(
        pool: &PgPool,
        agent_id: Uuid,
        name: &str,
        description: &str,
        output_schema: serde_json::Value,
    ) {
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, $3, $4, '{}'::jsonb, $5, 'network')"#,
            Uuid::now_v7(),
            agent_id,
            name,
            description,
            output_schema,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Search with dormant agents included: the seeded agents never poll,
    /// and most tests here are about filters, not the dormant rule (which
    /// has its own tests through `get_json`).
    async fn search_url(pool: PgPool, q: &str) -> serde_json::Value {
        let q = if q.contains("include_dormant") {
            q.to_owned()
        } else if q.is_empty() {
            "?include_dormant=true".to_owned()
        } else {
            format!("{q}&include_dormant=true")
        };
        get_json(
            crate::state::RelayState::new(pool, config()),
            &format!("/v1/discovery/agents{q}"),
        )
        .await
    }

    async fn get_json(state: crate::state::RelayState, uri: &str) -> serde_json::Value {
        let app = crate::router(state);
        let res = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    async fn heartbeat(pool: &PgPool, agent: Uuid, ago: &str) {
        sqlx::query(
            "INSERT INTO agent_heartbeats (agent_id, last_polled_at)
             VALUES ($1, now() - $2::interval)",
        )
        .bind(agent)
        .bind(ago)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn make_public(pool: &PgPool, agent: Uuid, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO agent_capabilities
                (id, agent_id, name, description, input_schema, output_schema, visibility,
                 public_invoke, public_monthly_quota_per_agent)
             VALUES ($1, $2, $3, 'callable by anyone', '{}'::jsonb, '{}'::jsonb, 'network', true, 10)",
        )
        .bind(id)
        .bind(agent)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    fn slugs(body: &serde_json::Value) -> Vec<String> {
        body["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["agent_slug"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Friends are counted live from accepted friendships.
    #[sqlx::test(migrations = "../migrations")]
    async fn friend_counts_are_live(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        let a = seed_agent(&pool, acct, "a", "A", "first", "pull", &[]).await;
        let b = seed_agent(&pool, acct, "b", "B", "second", "pull", &[]).await;
        let c = seed_agent(&pool, acct, "c", "C", "third", "pull", &[]).await;
        for (p, t, status) in [(a, b, "accepted"), (a, c, "proposed")] {
            sqlx::query(
                "INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(Uuid::now_v7())
            .bind(p)
            .bind(t)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        }
        let body = search_url(pool, "").await;
        let count = |slug: &str| {
            body["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["agent_slug"] == slug)
                .unwrap()["friend_count"]
                .clone()
        };
        assert_eq!(count("a"), 1);
        assert_eq!(count("b"), 1);
        assert_eq!(count("c"), 0, "a proposal isn't a friendship");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn dormant_agents_are_hidden_unless_asked_for(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        let online = seed_agent(&pool, acct, "online", "Online", "polls", "pull", &[]).await;
        let recent = seed_agent(
            &pool,
            acct,
            "recent",
            "Recent",
            "polled last week",
            "pull",
            &[],
        )
        .await;
        let stale = seed_agent(
            &pool,
            acct,
            "stale",
            "Stale",
            "polled long ago",
            "pull",
            &[],
        )
        .await;
        // Never polled: new ones count as seen when created, old ones are dormant.
        seed_agent(&pool, acct, "never", "Never", "never polled", "pull", &[]).await;
        let old = seed_agent(
            &pool,
            acct,
            "old",
            "Old",
            "never polled, made long ago",
            "pull",
            &[],
        )
        .await;
        sqlx::query("UPDATE agents SET created_at = now() - interval '40 days' WHERE id = $1")
            .bind(old)
            .execute(&pool)
            .await
            .unwrap();
        let public = seed_agent(
            &pool,
            acct,
            "public",
            "Public",
            "never polled, but callable",
            "pull",
            &[],
        )
        .await;
        seed_agent(
            &pool,
            acct,
            "pushy",
            "Pushy",
            "push agents never poll",
            "push",
            &[],
        )
        .await;
        heartbeat(&pool, online, "1 minute").await;
        heartbeat(&pool, recent, "7 days").await;
        heartbeat(&pool, stale, "40 days").await;
        make_public(&pool, public, "echo").await;

        let state = crate::state::RelayState::new(pool.clone(), config());
        let body = get_json(state.clone(), "/v1/discovery/agents").await;
        let mut shown = slugs(&body);
        shown.sort();
        assert_eq!(shown, ["never", "online", "public", "pushy", "recent"]);
        assert_eq!(body["total_estimate"], 5, "the count uses the same filter");

        let all = get_json(state, "/v1/discovery/agents?include_dormant=true").await;
        assert_eq!(all["agents"].as_array().unwrap().len(), 7);
        assert_eq!(all["total_estimate"], 7);

        let row = |slug: &str| {
            body["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["agent_slug"] == slug)
                .unwrap()
                .clone()
        };
        assert_eq!(row("online")["online"], true);
        assert_eq!(row("recent")["online"], false);
        assert_eq!(row("pushy")["online"], false);
        assert_eq!(row("online")["featured"], false, "no featured account set");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn the_featured_list_puts_the_house_agents_first(pool: PgPool) {
        let house = seed_account(&pool, "chakramcp", true).await;
        let other = seed_account(&pool, "someone", false).await;
        let guide = seed_agent(
            &pool,
            house,
            "guide",
            "ChakraMCP Guide",
            "answers",
            "pull",
            &[],
        )
        .await;
        let ask = make_public(&pool, guide, "guide.ask").await;
        // An online, callable agent elsewhere; an online one with nothing
        // public; an offline callable one.
        let echo = seed_agent(&pool, other, "echo", "Echo", "echoes", "pull", &[]).await;
        make_public(&pool, echo, "echo.say").await;
        heartbeat(&pool, echo, "30 seconds").await;
        let quiet = seed_agent(
            &pool,
            other,
            "quiet",
            "Quiet",
            "nothing public",
            "pull",
            &[],
        )
        .await;
        heartbeat(&pool, quiet, "30 seconds").await;
        let away = seed_agent(&pool, other, "away", "Away", "offline", "pull", &[]).await;
        make_public(&pool, away, "away.do").await;

        let state = crate::state::RelayState::new(pool.clone(), config())
            .with_featured_account(Some("chakramcp".into()));
        let body = get_json(state.clone(), "/v1/discovery/featured").await;
        assert_eq!(slugs(&body), ["guide", "echo"]);
        let first = &body["agents"][0];
        assert_eq!(first["featured"], true);
        assert_eq!(first["id"], guide.to_string());
        assert_eq!(first["public_capabilities"][0]["id"], ask.to_string());
        assert_eq!(first["public_capabilities"][0]["name"], "guide.ask");
        assert_eq!(body["agents"][1]["online"], true);

        // The main list marks them too.
        let main = get_json(state, "/v1/discovery/agents").await;
        let guide_row = main["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["agent_slug"] == "guide")
            .unwrap();
        assert_eq!(guide_row["featured"], true);

        // With no featured account, only online callable agents.
        let plain = get_json(
            crate::state::RelayState::new(pool, config()),
            "/v1/discovery/featured",
        )
        .await;
        assert_eq!(slugs(&plain), ["echo"]);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn empty_index_returns_empty_list(pool: PgPool) {
        let body = search_url(pool, "").await;
        assert_eq!(body["agents"].as_array().unwrap().len(), 0);
        assert_eq!(body["total_estimate"], 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn lists_network_agents(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        seed_agent(
            &pool,
            acct,
            "alice-bot",
            "Alice Bot",
            "Plans trips.",
            "pull",
            &[],
        )
        .await;
        seed_agent(
            &pool,
            acct,
            "bob-bot",
            "Bob Bot",
            "Books flights.",
            "pull",
            &[],
        )
        .await;

        let body = search_url(pool, "").await;
        let names: Vec<&str> = body["agents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["agent_slug"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"alice-bot"));
        assert!(names.contains(&"bob-bot"));
        assert_eq!(body["total_estimate"], 2);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn full_text_search_matches_display_name(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        seed_agent(
            &pool,
            acct,
            "alice-bot",
            "Alice Trip Planner",
            "Plans trips.",
            "pull",
            &[],
        )
        .await;
        seed_agent(
            &pool,
            acct,
            "bob-bot",
            "Bob Email Helper",
            "Sends email.",
            "pull",
            &[],
        )
        .await;

        let body = search_url(pool, "?q=trip").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "alice-bot");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn full_text_search_matches_capability_text(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        let alice = seed_agent(
            &pool,
            acct,
            "alice-bot",
            "Alice Bot",
            "Generic.",
            "pull",
            &[],
        )
        .await;
        add_capability(
            &pool,
            alice,
            "summarize_text",
            "Summarize a block of text.",
            serde_json::json!({}),
        )
        .await;
        let _bob = seed_agent(&pool, acct, "bob-bot", "Bob Bot", "Different.", "pull", &[]).await;

        let body = search_url(pool, "?q=summarize").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "alice-bot");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn capability_schema_jsonb_containment(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        let alice = seed_agent(&pool, acct, "alice", "Alice", "Plans trips.", "pull", &[]).await;
        add_capability(
            &pool,
            alice,
            "propose_slots",
            "Propose meeting slots.",
            serde_json::json!({"properties": {"slots": {"type": "array"}}}),
        )
        .await;
        let bob = seed_agent(&pool, acct, "bob", "Bob", "Sends email.", "pull", &[]).await;
        add_capability(
            &pool,
            bob,
            "send_email",
            "Send an email.",
            serde_json::json!({"properties": {"sent": {"type": "boolean"}}}),
        )
        .await;

        // URL-encoded JSON: {"properties":{"slots":{}}}
        let body = search_url(
            pool,
            "?capability_schema=%7B%22properties%22%3A%7B%22slots%22%3A%7B%7D%7D%7D",
        )
        .await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "alice");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn mode_filter_separates_push_and_pull(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        seed_agent(&pool, acct, "puller", "Puller", "Pulls.", "pull", &[]).await;
        seed_agent(&pool, acct, "pusher", "Pusher", "Pushes.", "push", &[]).await;

        let push = search_url(pool.clone(), "?mode=push").await;
        let push_agents = push["agents"].as_array().unwrap();
        assert_eq!(push_agents.len(), 1);
        assert_eq!(push_agents[0]["agent_slug"], "pusher");

        let pull = search_url(pool, "?mode=pull").await;
        let pull_agents = pull["agents"].as_array().unwrap();
        assert_eq!(pull_agents.len(), 1);
        assert_eq!(pull_agents[0]["agent_slug"], "puller");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn verified_filter(pool: PgPool) {
        let unv = seed_account(&pool, "unverified", false).await;
        let ver = seed_account(&pool, "verified", true).await;
        seed_agent(&pool, unv, "alice", "Alice", "x", "pull", &[]).await;
        seed_agent(&pool, ver, "bob", "Bob", "y", "pull", &[]).await;

        let body = search_url(pool, "?verified=true").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "bob");
        assert_eq!(agents[0]["verified"], true);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn tags_filter_requires_all(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        seed_agent(
            &pool,
            acct,
            "travel-bot",
            "Travel",
            "Plans.",
            "pull",
            &["travel", "booking"],
        )
        .await;
        seed_agent(
            &pool,
            acct,
            "email-bot",
            "Email",
            "Sends.",
            "pull",
            &["email"],
        )
        .await;

        let body = search_url(pool.clone(), "?tags=travel").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "travel-bot");

        let body2 = search_url(pool, "?tags=travel,booking").await;
        let agents2 = body2["agents"].as_array().unwrap();
        assert_eq!(agents2.len(), 1);
        assert_eq!(agents2[0]["agent_slug"], "travel-bot");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn excludes_tombstoned_agents(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        let alive = seed_agent(&pool, acct, "alive", "Alive", "x", "pull", &[]).await;
        let dead = seed_agent(&pool, acct, "dead", "Dead", "x", "pull", &[]).await;
        sqlx::query!(
            "UPDATE agents SET tombstoned_at = now() WHERE id = $1",
            dead
        )
        .execute(&pool)
        .await
        .unwrap();

        let body = search_url(pool, "").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "alive");
        let _ = alive;
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn excludes_private_agents(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        seed_agent(&pool, acct, "public-bot", "Public", "x", "pull", &[]).await;
        let private = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents (id, account_id, slug, display_name, visibility)
               VALUES ($1, $2, 'private-bot', 'Private', 'private')"#,
            private,
            acct,
        )
        .execute(&pool)
        .await
        .unwrap();

        let body = search_url(pool, "").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "public-bot");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn cursor_pagination_is_stable(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        for i in 0..5 {
            seed_agent(
                &pool,
                acct,
                &format!("agent-{i}"),
                &format!("Agent {i}"),
                "x",
                "pull",
                &[],
            )
            .await;
        }

        let page1 = search_url(pool.clone(), "?limit=2").await;
        let p1_agents = page1["agents"].as_array().unwrap();
        assert_eq!(p1_agents.len(), 2);
        let cursor = page1["next_cursor"].as_str().unwrap();

        let page2 = search_url(
            pool.clone(),
            &format!("?limit=2&cursor={}", urlencoding(cursor)),
        )
        .await;
        let p2_agents = page2["agents"].as_array().unwrap();
        assert_eq!(p2_agents.len(), 2);

        // No overlap.
        let p1_slugs: Vec<&str> = p1_agents
            .iter()
            .map(|a| a["agent_slug"].as_str().unwrap())
            .collect();
        let p2_slugs: Vec<&str> = p2_agents
            .iter()
            .map(|a| a["agent_slug"].as_str().unwrap())
            .collect();
        assert!(p1_slugs.iter().all(|s| !p2_slugs.contains(s)));

        // Page 1 has total_estimate; page 2 does not.
        assert_eq!(page1["total_estimate"], 5);
        assert!(page2.get("total_estimate").is_none());

        // Final page has no next_cursor.
        let cursor2 = page2["next_cursor"].as_str().unwrap();
        let page3 = search_url(pool, &format!("?limit=2&cursor={}", urlencoding(cursor2))).await;
        assert_eq!(page3["agents"].as_array().unwrap().len(), 1);
        assert!(page3["next_cursor"].is_null());
    }

    fn urlencoding(s: &str) -> String {
        // Cursors are URL-safe base64 with no special chars; passing
        // through is fine. This shim exists so the test reads cleanly.
        s.to_string()
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn returns_404_when_v2_disabled(pool: PgPool) {
        let mut cfg = config();
        cfg.discovery_v2_enabled = false;
        let app = crate::router(crate::state::RelayState::new(pool, cfg));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/discovery/agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn punctuation_only_q_does_not_500(pool: PgPool) {
        // Self-review #2: ?q=... reduces to an empty tsquery via
        // sanitization. Postgres `to_tsquery('simple', '')` errors,
        // so the handler MUST treat empty post-sanitization as "no
        // query" rather than passing the empty string through.
        seed_account(&pool, "acme", false).await;
        let body = search_url(pool, "?q=...").await;
        assert!(body["agents"].is_array());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_overlong_q(pool: PgPool) {
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let big = "x".repeat(600);
        let res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/discovery/agents?q={big}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_overlong_capability_schema(pool: PgPool) {
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        // 5 KB > MAX_CAP_SCHEMA_LEN (4 KB).
        let big = "%7B".to_string() + &"a".repeat(5_000) + "%7D";
        let res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/discovery/agents?capability_schema={big}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn rejects_malformed_capability_schema(pool: PgPool) {
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/discovery/agents?capability_schema=not%20json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
