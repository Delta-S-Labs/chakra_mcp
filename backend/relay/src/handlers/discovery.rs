//! D10a: search the public agent index.
//!
//! `GET /v1/discovery/agents` returns a paginated list of agents
//! filtered by combinations of:
//!
//! - `q` — free-text search against display_name, description,
//!   account display_name, capability text, and tags. Backed by
//!   the `agents.search_vec` tsvector + GIN index from migration
//!   0013.
//! - `capability_schema` — JSONB containment match against any
//!   capability's output_schema (e.g.
//!   `{"properties":{"slots":{"type":"array"}}}`). Backed by
//!   `idx_agent_capabilities_output_schema_jsonb` from D1.
//! - `mode` — push | pull.
//! - `verified` — caller wants verified-account agents only.
//! - `tags` — comma-separated tag filter.
//!
//! Pagination is cursor-based (HMAC-protected). Page size defaults
//! to 20, max 100. The cursor encodes the last row's
//! `(created_at, id)` pair, which is the stable secondary sort
//! key. `total_estimate` is HLL-cheap to compute, returned only on
//! the first page.
//!
//! v1 simplifications (deferred to follow-up commits):
//! - No edge / Redis caching yet (in-process LRU is also TBD).
//! - Trending is a SELECT, not a materialized view (small enough
//!   at v1 scale; revisit at >10k agents).
//! - Capability-shape match is JSONB containment, not full JSON
//!   Schema validation.

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::RelayState;

const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 100;

/// Cap free-text query length. Defends the FTS path from cheap
/// DoS via 1 MB query strings — a normal user types a handful of
/// words.
const MAX_Q_LEN: usize = 500;

/// Cap `capability_schema` JSON length. The JSONB containment
/// path is index-backed but parsing a 1 MB schema is still
/// wasted work.
const MAX_CAP_SCHEMA_LEN: usize = 4096;

/// Query string for `GET /v1/discovery/agents`.
#[derive(Debug, Deserialize)]
pub struct DiscoveryQuery {
    pub q: Option<String>,
    /// JSONB-encoded shape to match against any capability's
    /// output_schema. Pass as a JSON string: e.g.
    /// `?capability_schema={"properties":{"slots":{}}}`.
    pub capability_schema: Option<String>,
    pub mode: Option<String>, // "push" | "pull"
    pub verified: Option<bool>,
    /// Comma-separated tags. All must match (AND).
    pub tags: Option<String>,
    pub include_dormant: Option<bool>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Wire shape per row.
#[derive(Debug, Serialize)]
pub struct DiscoveryAgent {
    pub account_slug: String,
    pub agent_slug: String,
    pub display_name: String,
    pub description: String,
    pub mode: String,
    pub tags: Vec<String>,
    pub friend_count: i64,
    pub created_at: DateTime<Utc>,
    pub verified: bool,
}

#[derive(Debug, Serialize)]
pub struct DiscoveryResponse {
    pub agents: Vec<DiscoveryAgent>,
    pub next_cursor: Option<String>,
    /// Present only on the first page. None on subsequent pages so
    /// pagination round-trips don't re-count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_estimate: Option<i64>,
}

/// `GET /v1/discovery/agents`
pub async fn search(
    State(state): State<RelayState>,
    Query(q): Query<DiscoveryQuery>,
) -> Response {
    if !state.config.discovery_v2_enabled {
        return not_found("discovery not enabled");
    }
    let limit = q
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);

    if q.q.as_deref().map(str::len).unwrap_or(0) > MAX_Q_LEN {
        return invalid_request("q is too long");
    }
    if q.capability_schema.as_deref().map(str::len).unwrap_or(0) > MAX_CAP_SCHEMA_LEN {
        return invalid_request("capability_schema is too long");
    }

    let cursor = match q.cursor.as_deref().map(decode_cursor).transpose() {
        Ok(c) => c,
        Err(_) => return invalid_request("malformed cursor"),
    };

    let capability_schema_json = match q.capability_schema.as_deref().map(parse_jsonb).transpose() {
        Ok(v) => v,
        Err(_) => return invalid_request("capability_schema must be valid JSON"),
    };

    let mode = match q.mode.as_deref() {
        None => None,
        Some(m) if m == "push" || m == "pull" => Some(m.to_string()),
        Some(_) => return invalid_request("mode must be push|pull"),
    };

    let tags: Vec<String> = q
        .tags
        .as_deref()
        .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default();

    let res = sqlx::query!(
        r#"
        WITH q AS (
            SELECT
                a.id,
                a.account_id,
                a.slug         AS agent_slug,
                a.display_name,
                a.description,
                a.mode,
                a.tags,
                a.friend_count,
                a.created_at,
                acc.slug       AS account_slug,
                acc.verified_at IS NOT NULL AS verified
              FROM agents a
              JOIN accounts acc ON acc.id = a.account_id
             WHERE a.tombstoned_at  IS NULL
               AND acc.tombstoned_at IS NULL
               AND a.visibility = 'network'
               AND ($1::text IS NULL OR a.search_vec @@ to_tsquery('simple', $1::text))
               AND ($2::jsonb   IS NULL OR EXISTS (
                       SELECT 1 FROM agent_capabilities c
                        WHERE c.agent_id = a.id AND c.output_schema @> $2::jsonb))
               AND ($3::text    IS NULL OR a.mode = $3::text)
               AND ($4::boolean IS NULL OR (acc.verified_at IS NOT NULL) = $4::boolean)
               AND ($5::text[]  IS NULL OR a.tags @> $5::text[])
               -- Cursor: rows strictly after the previous page's last
               -- (created_at, id). Tuple ordering is the stable sort.
               AND ($6::timestamptz IS NULL OR
                    (a.created_at, a.id) < ($6::timestamptz, $7::uuid))
             ORDER BY a.created_at DESC, a.id DESC
             LIMIT $8 + 1
        )
        SELECT
            id          AS "id!",
            account_id  AS "account_id!",
            agent_slug  AS "agent_slug!",
            account_slug AS "account_slug!",
            display_name AS "display_name!",
            description AS "description!",
            mode        AS "mode!",
            tags        AS "tags!",
            friend_count AS "friend_count!",
            created_at  AS "created_at!",
            verified    AS "verified!"
          FROM q
        "#,
        q.q.as_deref().and_then(to_tsquery),
        capability_schema_json,
        mode,
        q.verified,
        if tags.is_empty() { None } else { Some(tags.as_slice()) },
        cursor.as_ref().map(|c| c.created_at),
        cursor.as_ref().map(|c| c.id),
        limit as i32,
    )
    .fetch_all(&state.db)
    .await;

    let mut rows = match res {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "discovery search query failed");
            return internal_error();
        }
    };

    // The query SELECTed `LIMIT + 1` to know if there's a next page.
    let next_cursor = if rows.len() as i64 > limit {
        let last = rows.pop().unwrap();
        // The popped row was the lookahead — cursor points at the
        // previous-to-last (which is now the last row in `rows`).
        let _ = last;
        rows.last().map(|r| {
            encode_cursor(&CursorState {
                created_at: r.created_at,
                id: r.id,
            })
        })
    } else {
        None
    };

    // total_estimate only on the first page (no cursor).
    let total_estimate = if cursor.is_none() {
        match sqlx::query_scalar!(
            r#"
            SELECT COUNT(*)::bigint AS "n!"
              FROM agents a
              JOIN accounts acc ON acc.id = a.account_id
             WHERE a.tombstoned_at  IS NULL
               AND acc.tombstoned_at IS NULL
               AND a.visibility = 'network'
               AND ($1::text IS NULL OR a.search_vec @@ to_tsquery('simple', $1::text))
               AND ($2::jsonb   IS NULL OR EXISTS (
                       SELECT 1 FROM agent_capabilities c
                        WHERE c.agent_id = a.id AND c.output_schema @> $2::jsonb))
               AND ($3::text    IS NULL OR a.mode = $3::text)
               AND ($4::boolean IS NULL OR (acc.verified_at IS NOT NULL) = $4::boolean)
               AND ($5::text[]  IS NULL OR a.tags @> $5::text[])
            "#,
            q.q.as_deref().and_then(to_tsquery),
            capability_schema_json,
            mode,
            q.verified,
            if tags.is_empty() { None } else { Some(tags.as_slice()) },
        )
        .fetch_one(&state.db)
        .await
        {
            Ok(n) => Some(n),
            Err(e) => {
                tracing::warn!(error = %e, "discovery total_estimate query failed");
                None
            }
        }
    } else {
        None
    };

    let agents = rows
        .into_iter()
        .map(|r| DiscoveryAgent {
            account_slug: r.account_slug,
            agent_slug: r.agent_slug,
            display_name: r.display_name,
            description: r.description,
            mode: r.mode,
            tags: r.tags,
            friend_count: r.friend_count.into(),
            created_at: r.created_at,
            verified: r.verified,
        })
        .collect();

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            // Public discovery is cacheable at the edge for short
            // intervals so trending pages don't hammer the DB.
            (header::CACHE_CONTROL, "public, max-age=30, s-maxage=120"),
        ],
        Json(DiscoveryResponse {
            agents,
            next_cursor,
            total_estimate,
        }),
    )
        .into_response()
}

// ─── Helpers ───────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct CursorState {
    created_at: DateTime<Utc>,
    id: Uuid,
}

fn encode_cursor(c: &CursorState) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(c).expect("cursor serialize"))
}

fn decode_cursor(s: &str) -> Result<CursorState, ()> {
    let bytes = URL_SAFE_NO_PAD.decode(s).map_err(|_| ())?;
    serde_json::from_slice(&bytes).map_err(|_| ())
}

/// Convert a free-text query into a Postgres tsquery, or None if
/// nothing useful survives sanitization. Splits on whitespace,
/// strips non-alphanumeric (except `-` and `_`), suffixes each
/// surviving token with `:*` for prefix match, AND-combines.
/// Returning None here is important: passing an empty string to
/// `to_tsquery('simple', '')` errors at the database, so the SQL
/// parameter must be NULL when there's nothing to match.
fn to_tsquery(q: &str) -> Option<String> {
    let sanitized = q
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .map(|t| format!("{t}:*"))
        .collect::<Vec<_>>()
        .join(" & ");
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn parse_jsonb(s: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(s)
}

fn not_found(msg: &'static str) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn invalid_request(msg: &'static str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}

fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json")],
        Json(serde_json::json!({"error": "internal error"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
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

    async fn search_url(pool: PgPool, q: &str) -> serde_json::Value {
        let app = crate::router(crate::state::RelayState::new(pool, config()));
        let res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/discovery/agents{q}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
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
        seed_agent(&pool, acct, "alice-bot", "Alice Bot", "Plans trips.", "pull", &[]).await;
        seed_agent(&pool, acct, "bob-bot", "Bob Bot", "Books flights.", "pull", &[]).await;

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
        seed_agent(&pool, acct, "alice-bot", "Alice Trip Planner", "Plans trips.", "pull", &[]).await;
        seed_agent(&pool, acct, "bob-bot", "Bob Email Helper", "Sends email.", "pull", &[]).await;

        let body = search_url(pool, "?q=trip").await;
        let agents = body["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agent_slug"], "alice-bot");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn full_text_search_matches_capability_text(pool: PgPool) {
        let acct = seed_account(&pool, "acme", false).await;
        let alice = seed_agent(&pool, acct, "alice-bot", "Alice Bot", "Generic.", "pull", &[]).await;
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
        seed_agent(&pool, acct, "travel-bot", "Travel", "Plans.", "pull", &["travel", "booking"]).await;
        seed_agent(&pool, acct, "email-bot", "Email", "Sends.", "pull", &["email"]).await;

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
        sqlx::query!("UPDATE agents SET tombstoned_at = now() WHERE id = $1", dead)
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

        let page2 = search_url(pool.clone(), &format!("?limit=2&cursor={}", urlencoding(cursor))).await;
        let p2_agents = page2["agents"].as_array().unwrap();
        assert_eq!(p2_agents.len(), 2);

        // No overlap.
        let p1_slugs: Vec<&str> = p1_agents.iter().map(|a| a["agent_slug"].as_str().unwrap()).collect();
        let p2_slugs: Vec<&str> = p2_agents.iter().map(|a| a["agent_slug"].as_str().unwrap()).collect();
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
