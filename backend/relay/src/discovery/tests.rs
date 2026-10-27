//! Discovery and addressing (MCP and discovery spec §5): ranked search,
//! the shared fields, profiles, standing, and caching by sign-in.

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use chakramcp_shared::config::SharedConfig;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const SECRET: &str = "test-secret";

fn config(discovery: bool) -> SharedConfig {
    SharedConfig {
        database_url: "ignored".into(),
        jwt_secret: SECRET.into(),
        admin_email: None,
        survey_enabled: false,
        frontend_base_url: "https://web.example".into(),
        app_base_url: "http://localhost:8080".into(),
        relay_base_url: "https://relay.example".into(),
        discovery_v2_enabled: discovery,
        log_filter: "warn".into(),
    }
}

/// A user, their individual account, and a token.
async fn user(pool: &PgPool, slug: &str) -> (Uuid, Uuid, String) {
    let user_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash) VALUES ($1, $2, 'U', 'x')",
    )
    .bind(user_id)
    .bind(format!("{user_id}@t.local"))
    .execute(pool)
    .await
    .unwrap();
    let account = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
         VALUES ($1, $2, $3, 'individual', $4)",
    )
    .bind(account)
    .bind(slug)
    .bind(format!("{slug} account"))
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
    )
    .bind(Uuid::now_v7())
    .bind(account)
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
    let token = chakramcp_shared::jwt::encode_jwt(
        &chakramcp_shared::jwt::UserClaims::new(user_id, format!("{user_id}@t.local"), false, 1),
        SECRET,
    )
    .unwrap();
    (user_id, account, token)
}

struct NewAgent<'a> {
    slug: &'a str,
    name: &'a str,
    description: &'a str,
    visibility: &'a str,
    tags: &'a [&'a str],
}

impl<'a> NewAgent<'a> {
    fn new(slug: &'a str) -> Self {
        Self {
            slug,
            name: "Agent",
            description: "Does things for people.",
            visibility: "network",
            tags: &[],
        }
    }
}

async fn agent(pool: &PgPool, account: Uuid, a: NewAgent<'_>) -> Uuid {
    let id = Uuid::now_v7();
    let tags: Vec<String> = a.tags.iter().map(|t| t.to_string()).collect();
    sqlx::query(
        "INSERT INTO agents (id, account_id, slug, display_name, description, visibility, tags)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(account)
    .bind(a.slug)
    .bind(a.name)
    .bind(a.description)
    .bind(a.visibility)
    .bind(&tags)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn capability(
    pool: &PgPool,
    agent: Uuid,
    name: &str,
    visibility: &str,
    public: bool,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agent_capabilities
            (id, agent_id, name, description, input_schema, output_schema, visibility,
             public_invoke, public_monthly_quota_per_agent)
         VALUES ($1, $2, $3, $4, '{\"type\":\"object\"}'::jsonb, '{}'::jsonb, $5, $6, $7)",
    )
    .bind(id)
    .bind(agent)
    .bind(name)
    .bind(format!("The {name} capability."))
    .bind(visibility)
    .bind(public)
    .bind(public.then_some(10))
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn grant(pool: &PgPool, granter: Uuid, grantee: Uuid, capability: Uuid) {
    sqlx::query(
        "INSERT INTO grants (id, granter_agent_id, grantee_agent_id, capability_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(granter)
    .bind(grantee)
    .bind(capability)
    .execute(pool)
    .await
    .unwrap();
}

async fn friends(pool: &PgPool, a: Uuid, b: Uuid, status: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO friendships (id, proposer_agent_id, target_agent_id, status) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(a)
    .bind(b)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn get(
    pool: &PgPool,
    discovery: bool,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, HeaderMap, Value) {
    let state = crate::state::RelayState::new(pool.clone(), config(discovery)).with_calls(
        crate::calls::CallsConfig {
            enabled: true,
            ..Default::default()
        },
    );
    let mut req = Request::builder().uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let res = crate::router(state)
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}

async fn search(pool: &PgPool, query: &str, token: Option<&str>) -> Value {
    let (status, _, body) = get(pool, true, &format!("/v1/discovery/agents?{query}"), token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn slugs(body: &Value) -> Vec<&str> {
    body["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["slug"].as_str().unwrap())
        .collect()
}

async fn tsquery(pool: &PgPool, q: &str) -> Option<String> {
    sqlx::query_scalar("SELECT discovery_query($1)::text")
        .bind(q)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ─── The query ───────────────────────────────────────────────

/// Words OR together, with English stems; three-letter words and longer
/// are prefixes; nothing usable is no query; nothing reaches tsquery syntax.
#[sqlx::test(migrations = "../migrations")]
async fn queries_are_words_ored(pool: PgPool) {
    assert_eq!(
        tsquery(&pool, "Booking AI").await.unwrap(),
        "'booking':* | 'book':* | 'ai'"
    );
    assert_eq!(tsquery(&pool, " ?? -- ").await, None);
    assert_eq!(
        tsquery(&pool, "o'reilly \\ :* & | !x").await.unwrap(),
        "'o' | 'reilly':* | 'reilli':* | 'x'"
    );
}

/// Any word matches; stems, prefixes and short words work.
#[sqlx::test(migrations = "../migrations")]
async fn search_matches_any_word_stems_and_prefixes(pool: PgPool) {
    let (_, acct, _) = user(&pool, "acme").await;
    for (slug, description) in [
        ("rooms", "We book rooms for teams."),
        ("calendar", "Keeps a calendar of events."),
        ("helper", "An AI helper for teams."),
        ("aid", "First aid instructions."),
    ] {
        agent(
            &pool,
            acct,
            NewAgent {
                description,
                ..NewAgent::new(slug)
            },
        )
        .await;
    }
    assert_eq!(slugs(&search(&pool, "q=booking", None).await), ["rooms"]);
    assert_eq!(slugs(&search(&pool, "q=calend", None).await), ["calendar"]);
    assert_eq!(
        slugs(&search(&pool, "q=ai", None).await),
        ["helper"],
        "short words match whole"
    );
    let body = search(&pool, "q=zebra+calendar+rooms", None).await;
    let mut any = slugs(&body);
    any.sort();
    assert_eq!(any, ["calendar", "rooms"]);
}

/// An exact slug beats a name match, which beats a tag match, which beats
/// a description match; more matched words rank higher.
#[sqlx::test(migrations = "../migrations")]
async fn search_ranks(pool: PgPool) {
    let (_, acct, _) = user(&pool, "acme").await;
    agent(
        &pool,
        acct,
        NewAgent {
            description: "A trip planner for you.",
            ..NewAgent::new("by-description")
        },
    )
    .await;
    agent(
        &pool,
        acct,
        NewAgent {
            tags: &["planner"],
            ..NewAgent::new("by-tag")
        },
    )
    .await;
    agent(
        &pool,
        acct,
        NewAgent {
            name: "Planner Pro",
            ..NewAgent::new("by-name")
        },
    )
    .await;
    agent(
        &pool,
        acct,
        NewAgent {
            name: "Zed",
            ..NewAgent::new("planner")
        },
    )
    .await;
    let body = search(&pool, "q=planner", None).await;
    assert_eq!(
        slugs(&body),
        ["planner", "by-name", "by-tag", "by-description"]
    );
    assert_eq!(body["total_estimate"], 4);

    agent(
        &pool,
        acct,
        NewAgent {
            description: "Travel and visas.",
            ..NewAgent::new("one-word")
        },
    )
    .await;
    agent(
        &pool,
        acct,
        NewAgent {
            description: "Travel booking and visas.",
            ..NewAgent::new("two-words")
        },
    )
    .await;
    assert_eq!(
        slugs(&search(&pool, "q=travel+booking", None).await),
        ["two-words", "one-word"]
    );

    // `account/slug` is exact too.
    assert_eq!(
        slugs(&search(&pool, "q=acme/by-tag", None).await)[0],
        "by-tag"
    );
}

/// Only network-visible capability text is searchable.
#[sqlx::test(migrations = "../migrations")]
async fn private_capability_text_isnt_searchable(pool: PgPool) {
    let (_, acct, _) = user(&pool, "acme").await;
    let a = agent(&pool, acct, NewAgent::new("open")).await;
    capability(&pool, a, "openthing", "network", false).await;
    capability(&pool, a, "secretsauce", "private", false).await;
    capability(&pool, a, "teamstuff", "org", false).await;
    assert_eq!(slugs(&search(&pool, "q=openthing", None).await), ["open"]);
    assert!(slugs(&search(&pool, "q=secretsauce", None).await).is_empty());
    assert!(slugs(&search(&pool, "q=teamstuff", None).await).is_empty());

    // A slug change re-indexes.
    sqlx::query("UPDATE agents SET slug = 'renamed-bot' WHERE id = $1")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        slugs(&search(&pool, "q=renamed", None).await),
        ["renamed-bot"]
    );
}

/// Ranked results page by offset; a cursor from another kind of search is
/// refused.
#[sqlx::test(migrations = "../migrations")]
async fn ranked_results_page_by_offset(pool: PgPool) {
    let (_, acct, _) = user(&pool, "acme").await;
    for i in 0..5 {
        let slug = format!("bot-{i}");
        agent(&pool, acct, NewAgent::new(&slug)).await;
    }
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..3 {
        let q = match &cursor {
            Some(c) => format!("q=bot&limit=2&cursor={c}"),
            None => "q=bot&limit=2".to_owned(),
        };
        let page = search(&pool, &q, None).await;
        seen.extend(slugs(&page).into_iter().map(str::to_owned));
        cursor = page["next_cursor"].as_str().map(str::to_owned);
    }
    assert!(cursor.is_none());
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 5);

    let page = search(&pool, "q=bot&limit=2", None).await;
    let ranked_cursor = page["next_cursor"].as_str().unwrap();
    let (status, _, body) = get(
        &pool,
        true,
        &format!("/v1/discovery/agents?limit=2&cursor={ranked_cursor}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
}

/// The capability filter is a case-insensitive part of a visible name.
#[sqlx::test(migrations = "../migrations")]
async fn capability_filter_matches_part_of_a_name(pool: PgPool) {
    let (_, acct, _) = user(&pool, "acme").await;
    let a = agent(&pool, acct, NewAgent::new("booker")).await;
    capability(&pool, a, "hold_room", "network", false).await;
    let b = agent(&pool, acct, NewAgent::new("hider")).await;
    capability(&pool, b, "hold_secret", "private", false).await;
    assert_eq!(
        slugs(&search(&pool, "capability=HOLD", None).await),
        ["booker"]
    );
}

// ─── Signed in ───────────────────────────────────────────────

/// With a token, search adds your private agents and your standing, and
/// isn't cached; `callable` narrows to what you can call; a bad token is
/// 401.
#[sqlx::test(migrations = "../migrations")]
async fn signed_in_search(pool: PgPool) {
    let (_, alice_acct, alice) = user(&pool, "alice").await;
    let (_, bob_acct, _) = user(&pool, "bob").await;
    let mine = agent(
        &pool,
        alice_acct,
        NewAgent {
            visibility: "private",
            ..NewAgent::new("mine")
        },
    )
    .await;
    let granted = agent(&pool, bob_acct, NewAgent::new("granted")).await;
    let open = agent(&pool, bob_acct, NewAgent::new("open")).await;
    agent(&pool, bob_acct, NewAgent::new("closed")).await;
    agent(
        &pool,
        bob_acct,
        NewAgent {
            visibility: "private",
            ..NewAgent::new("bobs-secret")
        },
    )
    .await;
    let hold = capability(&pool, granted, "hold", "network", false).await;
    grant(&pool, granted, mine, hold).await;
    capability(&pool, open, "ask", "network", true).await;

    let (status, headers, body) = get(&pool, true, "/v1/discovery/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=30, s-maxage=120"
    );
    assert_eq!(headers[header::VARY], "Authorization");
    let mut shown = slugs(&body);
    shown.sort();
    assert_eq!(shown, ["closed", "granted", "open"]);
    assert!(body["agents"][0].get("you").is_none());

    let (_, headers, body) = get(&pool, true, "/v1/discovery/agents", Some(&alice)).await;
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
    let mut shown = slugs(&body);
    shown.sort();
    assert_eq!(shown, ["closed", "granted", "mine", "open"]);
    let row = |slug: &str| {
        body["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["slug"] == slug)
            .unwrap()
            .clone()
    };
    assert_eq!(row("mine")["visibility"], "private");
    assert_eq!(row("granted")["you"]["callable"][0]["via"], "grant");
    assert_eq!(
        row("granted")["you"]["callable"][0]["as_agent_id"],
        mine.to_string()
    );
    assert_eq!(row("open")["you"]["callable"][0]["via"], "public");
    assert_eq!(row("open")["capabilities"][0]["access"], "public");
    assert_eq!(row("open")["has_public_capabilities"], true);
    assert_eq!(row("closed")["you"]["callable"], serde_json::json!([]));

    let mut callable = slugs(&search(&pool, "callable=true", Some(&alice)).await)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    callable.sort();
    assert_eq!(callable, ["granted", "open"]);
    let (status, _, _) = get(&pool, true, "/v1/discovery/agents?callable=true", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, headers, _) = get(&pool, true, "/v1/discovery/agents", Some("nope")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.contains_key(header::WWW_AUTHENTICATE));
}

// ─── Profiles and standing ───────────────────────────────────

/// A profile by name or id: shared fields, owner, capabilities in full,
/// reviews, how to call; a renamed agent's old name still finds it.
#[sqlx::test(migrations = "../migrations")]
async fn profiles(pool: PgPool) {
    let (_, bob_acct, _) = user(&pool, "bob").await;
    let (_, carol_acct, _) = user(&pool, "carol").await;
    let venue = agent(
        &pool,
        bob_acct,
        NewAgent {
            name: "Venue bot",
            tags: &["events"],
            ..NewAgent::new("venue-bot")
        },
    )
    .await;
    capability(&pool, venue, "check_availability", "network", true).await;
    capability(&pool, venue, "internal", "private", false).await;
    let fan = agent(
        &pool,
        carol_acct,
        NewAgent {
            name: "Carol's fan",
            ..NewAgent::new("fan")
        },
    )
    .await;
    let shy = agent(
        &pool,
        carol_acct,
        NewAgent {
            name: "Shy",
            visibility: "private",
            ..NewAgent::new("shy")
        },
    )
    .await;
    for (reviewer, rating, text) in [(fan, 5, "Great"), (shy, 3, "Fine")] {
        sqlx::query(
            "INSERT INTO agent_reviews (id, reviewer_agent_id, target_agent_id, rating, comment, tier)
             VALUES ($1, $2, $3, $4, $5, 'public')",
        )
        .bind(Uuid::now_v7())
        .bind(reviewer)
        .bind(venue)
        .bind(rating as i16)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    }

    let (status, headers, p) = get(&pool, true, "/v1/discovery/agents/bob/venue-bot", None).await;
    assert_eq!(status, StatusCode::OK, "{p}");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=30, s-maxage=120"
    );
    assert_eq!(p["id"], venue.to_string());
    assert_eq!(p["slug"], "venue-bot");
    assert_eq!(p["tags"], serde_json::json!(["events"]));
    assert_eq!(p["owner"]["account_slug"], "bob");
    assert_eq!(p["owner"]["display_name"], "bob account");
    let caps = p["capabilities"].as_array().unwrap();
    assert_eq!(caps.len(), 1, "private capabilities aren't shown");
    assert_eq!(caps[0]["name"], "check_availability");
    assert_eq!(caps[0]["access"], "public");
    assert_eq!(caps[0]["public_monthly_quota_per_agent"], 10);
    assert_eq!(caps[0]["input_schema"]["type"], "object");
    assert_eq!(p["reviews"]["count"], 2);
    let by: Vec<&str> = p["reviews"]["latest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["by"].as_str().unwrap())
        .collect();
    assert_eq!(by, ["An agent", "Carol's fan"]);
    assert_eq!(
        p["how_to_call"]["cli"],
        "chakramcp invoke ensure bob/venue-bot check_availability '{}' --from <your-agent> --wait"
    );
    assert_eq!(
        p["how_to_call"]["a2a_card"],
        "https://relay.example/agents/bob/venue-bot/.well-known/agent-card.json"
    );
    assert_eq!(
        p["how_to_call"]["profile"],
        "https://web.example/agents/bob/venue-bot"
    );
    assert!(p.get("you").is_none());

    let (status, _, by_id) = get(&pool, true, &format!("/v1/discovery/agents/{venue}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(by_id["slug"], "venue-bot");

    sqlx::query(
        "INSERT INTO slug_aliases (scope, account_id, old_slug, new_slug, expires_at)
         VALUES ('agent', $1, 'old-venue', 'venue-bot', now() + interval '90 days')",
    )
    .bind(bob_acct)
    .execute(&pool)
    .await
    .unwrap();
    let (status, _, renamed) = get(&pool, true, "/v1/discovery/agents/bob/old-venue", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["slug"], "venue-bot");

    for missing in [
        "/v1/discovery/agents/bob/nobody",
        "/v1/discovery/agents/nobody/venue-bot",
        "/v1/discovery/agents/not-an-id",
    ] {
        let (status, _, _) = get(&pool, true, missing, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    }
}

/// Private and org agents' profiles are for members (and org-sharers);
/// with `DISCOVERY_V2` off, profiles need sign-in.
#[sqlx::test(migrations = "../migrations")]
async fn who_sees_a_profile(pool: PgPool) {
    let (alice_id, alice_acct, alice) = user(&pool, "alice").await;
    let (colleague_id, _, colleague) = user(&pool, "colleague").await;
    let (_, _, stranger) = user(&pool, "stranger").await;
    agent(
        &pool,
        alice_acct,
        NewAgent {
            visibility: "private",
            ..NewAgent::new("secret")
        },
    )
    .await;
    agent(
        &pool,
        alice_acct,
        NewAgent {
            visibility: "org",
            ..NewAgent::new("team")
        },
    )
    .await;
    agent(&pool, alice_acct, NewAgent::new("open")).await;
    let guild = Uuid::now_v7();
    sqlx::query("INSERT INTO accounts (id, slug, display_name, account_type) VALUES ($1, 'guild', 'Guild', 'organization')")
        .bind(guild)
        .execute(&pool)
        .await
        .unwrap();
    for u in [alice_id, colleague_id] {
        sqlx::query("INSERT INTO account_memberships (id, account_id, user_id, role) VALUES ($1, $2, $3, 'member')")
            .bind(Uuid::now_v7())
            .bind(guild)
            .bind(u)
            .execute(&pool)
            .await
            .unwrap();
    }

    let status = |path: &'static str, token: Option<String>, discovery: bool| {
        let pool = pool.clone();
        async move { get(&pool, discovery, path, token.as_deref()).await.0 }
    };
    let secret = "/v1/discovery/agents/alice/secret";
    let team = "/v1/discovery/agents/alice/team";
    let open = "/v1/discovery/agents/alice/open";
    assert_eq!(status(secret, None, true).await, StatusCode::NOT_FOUND);
    assert_eq!(
        status(secret, Some(stranger.clone()), true).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(secret, Some(colleague.clone()), true).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(secret, Some(alice.clone()), true).await,
        StatusCode::OK
    );
    assert_eq!(
        status(team, Some(stranger.clone()), true).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(team, Some(colleague.clone()), true).await,
        StatusCode::OK
    );
    assert_eq!(status(open, None, true).await, StatusCode::OK);

    assert_eq!(status(open, None, false).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status(open, Some(stranger), false).await, StatusCode::OK);
    let (status, _, _) = get(&pool, false, "/v1/discovery/agents", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "search stays off");
}

/// Standing: callable by grant or publicly, friendships, and who could
/// start a call; a scoped credential sees only its agents' standing.
#[sqlx::test(migrations = "../migrations")]
async fn standing(pool: PgPool) {
    let (alice_id, alice_acct, alice) = user(&pool, "alice").await;
    let (_, bob_acct, _) = user(&pool, "bob").await;
    let planner = agent(&pool, alice_acct, NewAgent::new("planner")).await;
    let helper = agent(&pool, alice_acct, NewAgent::new("helper")).await;
    let venue = agent(&pool, bob_acct, NewAgent::new("venue")).await;
    let hold = capability(&pool, venue, "hold_room", "network", false).await;
    let check = capability(&pool, venue, "check", "network", true).await;
    capability(&pool, venue, "cancel", "network", false).await;
    grant(&pool, venue, planner, hold).await;
    let friendship = friends(&pool, planner, venue, "accepted").await;
    friends(&pool, helper, venue, "rejected").await;

    let (status, headers, you) = get(
        &pool,
        true,
        "/v1/discovery/standing?agent=bob/venue",
        Some(&alice),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{you}");
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
    let callable: Vec<(&str, &str)> = you["callable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["capability"].as_str().unwrap(),
                c["via"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(callable, [("hold_room", "grant"), ("check", "public")]);
    assert_eq!(you["callable"][0]["capability_id"], hold.to_string());
    assert_eq!(you["callable"][1]["capability_id"], check.to_string());
    assert_eq!(you["friendships"].as_array().unwrap().len(), 1);
    assert_eq!(you["friendships"][0]["id"], friendship.to_string());
    assert_eq!(you["friendships"][0]["your_agent_id"], planner.to_string());
    assert_eq!(
        you["can_start_call_as"],
        serde_json::json!([planner.to_string()])
    );

    // The same by id, and in the profile.
    let (_, _, by_id) = get(
        &pool,
        true,
        &format!("/v1/discovery/standing?agent={venue}"),
        Some(&alice),
    )
    .await;
    assert_eq!(by_id, you);
    let (_, _, profile) = get(&pool, true, "/v1/discovery/agents/bob/venue", Some(&alice)).await;
    assert_eq!(profile["you"], you);

    // A key scoped to `helper` alone: no grant, no friendship, no calls.
    let key_id = Uuid::now_v7();
    let key = format!("ck_scoped_{key_id}");
    sqlx::query(
        "INSERT INTO api_keys (id, user_id, key_hash, name, key_prefix) VALUES ($1, $2, $3, 'scoped', 'ck_scoped')",
    )
    .bind(key_id)
    .bind(alice_id)
    .bind(hex::encode(<sha2::Sha256 as sha2::Digest>::digest(key.as_bytes())))
    .execute(&pool)
    .await
    .unwrap();
    let scope = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO credential_scopes (id, cred_kind, cred_ref, user_id, agent_scope)
         VALUES ($1, 'api_key', $2, $3, 'selected')",
    )
    .bind(scope)
    .bind(key_id.to_string())
    .bind(alice_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO credential_scope_agents (credential_scope_id, agent_id) VALUES ($1, $2)",
    )
    .bind(scope)
    .bind(helper)
    .execute(&pool)
    .await
    .unwrap();
    let (_, _, scoped) = get(
        &pool,
        true,
        "/v1/discovery/standing?agent=bob/venue",
        Some(&key),
    )
    .await;
    let callable: Vec<&str> = scoped["callable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["via"].as_str().unwrap())
        .collect();
    assert_eq!(callable, ["public"]);
    assert_eq!(scoped["friendships"], serde_json::json!([]));
    assert_eq!(scoped["can_start_call_as"], serde_json::json!([]));

    let (status, _, _) = get(&pool, true, "/v1/discovery/standing?agent=bob/venue", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = get(
        &pool,
        true,
        "/v1/discovery/standing?agent=venue",
        Some(&alice),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ─── The network list ────────────────────────────────────────

/// The network list keeps its fields and gains the shared ones.
#[sqlx::test(migrations = "../migrations")]
async fn the_network_list_keeps_its_fields(pool: PgPool) {
    let (_, alice_acct, alice) = user(&pool, "alice").await;
    let a = agent(&pool, alice_acct, NewAgent::new("mine")).await;
    capability(&pool, a, "ask", "network", true).await;
    let (status, _, body) = get(&pool, true, "/v1/network/agents", Some(&alice)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = body["agents"][0].as_object().unwrap();
    for key in [
        "id",
        "account_id",
        "account_slug",
        "account_display_name",
        "slug",
        "display_name",
        "description",
        "mode",
        "accept_calls",
        "tags",
        "verified",
        "endpoint_url",
        "created_at",
        "updated_at",
        "is_mine",
        "capability_count",
        "avg_rating",
        "review_count",
        "online",
        // Shared fields it gains.
        "capabilities",
        "last_seen_at",
        "friend_count",
        "featured",
        "visibility",
    ] {
        assert!(row.contains_key(key), "missing {key}: {body}");
    }
    assert_eq!(row["is_mine"], true);
    assert_eq!(row["capability_count"], 1);
    assert_eq!(row["capabilities"][0]["name"], "ask");
}
