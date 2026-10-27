//! Discovery, hermetic: `httpmock` serves responses recorded from the
//! relay (`sdks/testdata/discovery`). Also tags on agent create and
//! update, and the shared listing fields on the network list.

use chakramcp::{
    AcceptCalls, Access, AgentMode, CallableVia, ChakraMCP, CreateAgentRequest, DiscoveryQuery,
    FriendshipStatus, NetworkPage, UpdateAgentRequest, Visibility,
};
use chrono::{DateTime, Utc};
use httpmock::prelude::*;
use serde_json::{json, Value};

fn build(server: &MockServer) -> ChakraMCP {
    ChakraMCP::builder()
        .api_key("ck_test")
        .app_url(server.base_url())
        .relay_url(server.base_url())
        .build()
        .expect("client builds")
}

/// A recorded response, as the relay sent it.
fn fixture(name: &str) -> String {
    let path = format!(
        "{}/../testdata/discovery/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn at(s: &str) -> DateTime<Utc> {
    s.parse().expect("timestamp")
}

#[tokio::test]
async fn search_sends_its_filters_and_decodes_a_page() {
    let server = MockServer::start_async().await;
    let filtered = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/discovery/agents")
                .query_param("q", "guide")
                .query_param("tags", "help,docs")
                .query_param("callable", "true")
                .query_param("capability_schema", r#"{"properties":{"answer":{}}}"#)
                .query_param("mode", "pull")
                .query_param("limit", "2")
                .query_param_missing("verified")
                .query_param_missing("include_dormant")
                .query_param_missing("cursor");
            then.status(200)
                .header("content-type", "application/json")
                .body(fixture("search"));
        })
        .await;
    let bare = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/discovery/agents")
                .is_true(|req| req.uri().query().is_none());
            then.status(200)
                .json_body(json!({ "agents": [], "next_cursor": null }));
        })
        .await;
    let chakra = build(&server);

    let page = chakra
        .discovery()
        .search(&DiscoveryQuery {
            q: Some("guide".into()),
            tags: vec!["help".into(), "docs".into()],
            callable: Some(true),
            capability_schema: Some(json!({ "properties": { "answer": {} } })),
            mode: Some(AgentMode::Pull),
            limit: Some(2),
            ..Default::default()
        })
        .await
        .expect("search");
    filtered.assert_async().await;
    assert_eq!(page.total_estimate, Some(1));
    assert_eq!(page.next_cursor, None);
    let guide = &page.agents[0];
    let a = &guide.listing;
    assert_eq!(
        (a.slug.as_str(), a.account_slug.as_str()),
        ("guide", "chakramcp")
    );
    assert_eq!(a.mode, AgentMode::Pull);
    assert_eq!(a.visibility, Visibility::Network);
    assert_eq!(a.accept_calls, AcceptCalls::Friends);
    assert!(a.online && a.featured && !a.verified);
    assert_eq!(a.last_seen_at, at("2026-10-27T09:41:14.192415Z"));
    assert_eq!((a.friend_count, a.avg_rating, a.review_count), (0, None, 0));
    let cap = &guide.capabilities[0];
    assert_eq!(
        (cap.name.as_str(), cap.access),
        ("guide.ask", Access::Public)
    );
    assert_eq!(guide.agent_slug, "guide");
    assert!(guide.has_public_capabilities);
    assert!(guide.you.is_none());

    // No filters, no query string.
    let empty = chakra
        .discovery()
        .search(&DiscoveryQuery::default())
        .await
        .expect("bare search");
    bare.assert_async().await;
    assert!(empty.agents.is_empty() && empty.total_estimate.is_none());
}

#[tokio::test]
async fn featured_returns_the_agents() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/v1/discovery/featured");
            then.status(200)
                .header("content-type", "application/json")
                .body(fixture("featured"));
        })
        .await;

    let agents = build(&server)
        .discovery()
        .featured()
        .await
        .expect("featured");
    mock.assert_async().await;
    let slugs: Vec<_> = agents.iter().map(|a| a.listing.slug.as_str()).collect();
    assert_eq!(slugs, ["guide", "ping"]);
    assert!(agents.iter().all(|a| a.listing.featured));
    assert_eq!(agents[1].capabilities[0].name, "ping.echo");
    assert_eq!(agents[0].public_capabilities[0].name, "guide.ask");
}

#[tokio::test]
async fn profile_takes_an_id_or_account_and_agent() {
    let server = MockServer::start_async().await;
    let by_name = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/discovery/agents/chakramcp/guide");
            then.status(200)
                .header("content-type", "application/json")
                .body(fixture("profile"));
        })
        .await;
    let by_id = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/discovery/agents/01a1607c-5ca5-727e-96b8-4fcc3b4a18f7");
            then.status(200)
                .header("content-type", "application/json")
                .body(fixture("profile"));
        })
        .await;
    let chakra = build(&server);

    let profile = chakra
        .discovery()
        .profile("chakramcp/guide")
        .await
        .expect("profile by name");
    chakra
        .discovery()
        .profile("01a1607c-5ca5-727e-96b8-4fcc3b4a18f7")
        .await
        .expect("profile by id");
    by_name.assert_async().await;
    by_id.assert_async().await;

    assert_eq!(profile.listing.display_name, "ChakraMCP Guide");
    assert_eq!(
        (profile.listing.avg_rating, profile.listing.review_count),
        (Some(5.0), 1)
    );
    assert_eq!(profile.owner.account_slug, "chakramcp");
    let cap = &profile.capabilities[0];
    assert_eq!(cap.access, Access::Public);
    assert!(cap.input_schema.is_object());
    assert_eq!(cap.input_schema["required"], json!(["question"]));
    assert_eq!(cap.public_monthly_quota_per_agent, Some(300));
    assert!(!cap.consent_required);
    assert_eq!(cap.semantics, "autonomous");
    assert_eq!(
        (profile.reviews.avg_rating, profile.reviews.count),
        (Some(5.0), 1)
    );
    let review = &profile.reviews.latest[0];
    assert_eq!((review.rating, review.by.as_str()), (5, "An agent"));
    assert_eq!(review.text.as_deref(), Some("Clear answers, fast."));
    assert_eq!(review.at, at("2026-10-27T09:00:00Z"));
    assert!(profile
        .how_to_call
        .cli
        .contains("chakramcp invoke ensure chakramcp/guide"));
    assert!(profile.how_to_call.a2a_card.is_some());
    assert_eq!(profile.how_to_call.mcp, None);
    assert!(profile.you.is_none());
}

#[tokio::test]
async fn standing_names_the_agent_in_the_query() {
    let server = MockServer::start_async().await;
    let mock = server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/v1/discovery/standing")
                .is_true(|req| req.uri().query() == Some("agent=bob%2Fvenue-bot"));
            then.status(200)
                .header("content-type", "application/json")
                .body(fixture("standing"));
        })
        .await;

    let you = build(&server)
        .discovery()
        .standing("bob/venue-bot")
        .await
        .expect("standing");
    mock.assert_async().await;
    let via: Vec<_> = you.callable.iter().map(|c| c.via).collect();
    assert_eq!(via, [CallableVia::Grant, CallableVia::Public]);
    assert_eq!(
        you.callable[0].as_agent_id.as_deref(),
        Some("019a2b3c-0000-7000-8000-0000000000a1")
    );
    assert_eq!(you.callable[1].as_agent_id, None);
    assert_eq!(you.callable[1].capability, "check_availability");
    assert_eq!(you.friendships[0].status, FriendshipStatus::Accepted);
    assert_eq!(you.can_start_call_as.len(), 1);
}

#[tokio::test]
async fn agents_create_and_update_send_tags() {
    let server = MockServer::start_async().await;
    let agent = json!({
        "id": "a1", "account_id": "acc1", "account_slug": "alice",
        "account_display_name": "Alice", "slug": "rooms", "display_name": "Rooms",
        "description": "", "visibility": "private", "endpoint_url": null,
        "created_at": "2026-10-27T10:00:00Z", "updated_at": "2026-10-27T10:00:00Z",
        "is_mine": true, "capability_count": 0, "avg_rating": null, "review_count": 0,
        "accept_calls": "friends", "tags": ["booking"],
    });
    let create = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/agents").json_body(json!({
                "account_id": "acc1", "slug": "rooms", "display_name": "Rooms",
                "tags": ["booking"],
            }));
            then.status(200).json_body(agent.clone());
        })
        .await;
    let clear = server
        .mock_async(|when, then| {
            when.method(PATCH)
                .path("/v1/agents/a1")
                .json_body(json!({ "tags": [] }));
            then.status(200).json_body(agent.clone());
        })
        .await;
    let chakra = build(&server);

    let created = chakra
        .agents()
        .create(&CreateAgentRequest {
            account_id: "acc1".into(),
            slug: "rooms".into(),
            display_name: "Rooms".into(),
            tags: vec!["booking".into()],
            ..Default::default()
        })
        .await
        .expect("creates");
    create.assert_async().await;
    assert_eq!(created.tags, ["booking"]);

    // `Some(vec![])` clears the tags; `None` (other tests) leaves them out.
    chakra
        .agents()
        .update(
            "a1",
            &UpdateAgentRequest {
                tags: Some(vec![]),
                ..Default::default()
            },
        )
        .await
        .expect("updates");
    clear.assert_async().await;
}

#[test]
fn network_agents_carry_the_listing_fields() {
    // The relay's network list is a search result's fields plus these.
    let search: Value = serde_json::from_str(&fixture("search")).unwrap();
    let mut agent = search["agents"][0].as_object().unwrap().clone();
    let extra = json!({
        "account_id": "acc1", "account_display_name": "ChakraMCP", "endpoint_url": null,
        "updated_at": "2026-10-21T22:02:05.350157Z", "is_mine": false, "capability_count": 1,
    });
    agent.extend(extra.as_object().unwrap().clone());
    let page: NetworkPage =
        serde_json::from_value(json!({ "agents": [agent], "next_cursor": null })).unwrap();
    let a = &page.agents[0];
    assert_eq!(a.visibility, Some(Visibility::Network));
    assert_eq!(a.capabilities[0].access, Access::Public);
    assert_eq!(a.last_seen_at, Some(at("2026-10-27T09:41:14.192415Z")));
    assert!(a.featured);
    assert_eq!(a.friend_count, 0);

    // A relay before 0.6.0 sends none of them.
    for key in [
        "visibility",
        "capabilities",
        "last_seen_at",
        "friend_count",
        "featured",
    ] {
        agent.remove(key);
    }
    let old: NetworkPage = serde_json::from_value(json!({ "agents": [agent] })).unwrap();
    let a = &old.agents[0];
    assert_eq!((a.visibility, a.last_seen_at), (None, None));
    assert!(a.capabilities.is_empty() && !a.featured);
}
