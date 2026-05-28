//! Agent-to-agent ratings & reviews. Sub-project 2 of the ratings
//! feature; see
//! `docs/superpowers/specs/2026-05-28-agent-ratings-and-reviews-design.md`.
//!
//! One review per (reviewer_agent, target_agent), editable in place
//! (no hard delete). 1–5 stars + optional comment + ≥1 tagged
//! capability the reviewer has actually invoked (proven via
//! `relay_invocations.status != 'rejected'`). Tier ('friend' /
//! 'public') is stamped at write-time so the label doesn't drift if
//! the friendship state changes later. Soft-hide by the target's
//! owner.

use axum::extract::{Path, Query, State};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::{user_is_member, AuthUser};
use crate::handlers::friendships::AgentSummary;
use crate::state::RelayState;

const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 100;
const MAX_COMMENT_LEN: usize = 4_000;

// ─── DTOs ────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewTagDto {
    pub capability_id: Uuid,
    pub capability_name: String,
}

#[derive(Debug, Serialize)]
pub struct ReviewDto {
    pub id: Uuid,
    pub reviewer: AgentSummary,
    pub target: AgentSummary,
    pub rating: i16,
    pub comment: Option<String>,
    /// 'friend' or 'public'. Stamped at write-time from the
    /// relationship state then; doesn't drift later.
    pub tier: String,
    pub tags: Vec<ReviewTagDto>,
    pub hidden: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// True when the requesting user owns the reviewer agent.
    pub i_authored: bool,
}

#[derive(Debug, Deserialize)]
pub struct WriteReviewRequest {
    pub reviewer_agent_id: Uuid,
    pub rating: i16,
    #[serde(default)]
    pub comment: Option<String>,
    pub tagged_capability_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ReviewListResponse {
    pub reviews: Vec<ReviewDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub summary: ReviewSummary,
}

#[derive(Debug, Serialize)]
pub struct ReviewSummary {
    /// Mean rating over the *visible* set. `None` when `count = 0`.
    pub average: Option<f64>,
    /// Number of reviews in the *visible* set.
    pub count: i64,
    pub distribution: ReviewDistribution,
}

#[derive(Debug, Serialize)]
pub struct ReviewDistribution {
    #[serde(rename = "1")]
    pub one: i64,
    #[serde(rename = "2")]
    pub two: i64,
    #[serde(rename = "3")]
    pub three: i64,
    #[serde(rename = "4")]
    pub four: i64,
    #[serde(rename = "5")]
    pub five: i64,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub tier: Option<String>,
    pub include_hidden: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct EligibilityResponse {
    pub eligible: Vec<EligibleReviewer>,
}

#[derive(Debug, Serialize)]
pub struct EligibleReviewer {
    pub reviewer_agent_id: Uuid,
    pub reviewer_display_name: String,
    pub tagable_capability_ids: Vec<Uuid>,
}

// ─── Helpers ─────────────────────────────────────────────

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

/// True iff there is an accepted friendship between `a` and `b` in
/// either direction. Used to stamp the `tier` column on a review.
async fn have_accepted_friendship(db: &PgPool, a: Uuid, b: Uuid) -> Result<bool, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM friendships
             WHERE status = 'accepted'
               AND (
                    (proposer_agent_id = $1 AND target_agent_id = $2)
                 OR (proposer_agent_id = $2 AND target_agent_id = $1)
               )
        ) AS "yes!"
        "#,
        a,
        b,
    )
    .fetch_one(db)
    .await?;
    Ok(row.yes)
}

/// Fetch the target agent + the caller's tombstone/membership flags.
/// Returns `NotFound` for a missing or tombstoned agent (which also
/// hides the existence of private agents from non-members).
async fn resolve_target(state: &RelayState, target_id: Uuid) -> Result<(), ApiError> {
    let row = sqlx::query!(
        r#"SELECT id FROM agents WHERE id = $1 AND tombstoned_at IS NULL"#,
        target_id,
    )
    .fetch_optional(&state.db)
    .await?;
    if row.is_none() {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

// ─── POST /v1/agents/{target}/reviews ────────────────────

pub async fn write(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(target_agent_id): Path<Uuid>,
    Json(req): Json<WriteReviewRequest>,
) -> ApiResult<Json<ReviewDto>> {
    // 1. Sanity bounds first — cheap, friendly errors before any DB I/O.
    if !(1..=5).contains(&req.rating) {
        return Err(ApiError::InvalidRequest(
            "rating must be between 1 and 5".into(),
        ));
    }
    if let Some(c) = req.comment.as_deref() {
        if c.len() > MAX_COMMENT_LEN {
            return Err(ApiError::InvalidRequest(format!(
                "comment must be at most {MAX_COMMENT_LEN} characters"
            )));
        }
    }
    if req.tagged_capability_ids.is_empty() {
        return Err(ApiError::InvalidRequest(
            "at least one tagged_capability_ids entry is required".into(),
        ));
    }
    if req.reviewer_agent_id == target_agent_id {
        return Err(ApiError::InvalidRequest(
            "cannot review your own agent (reviewer_agent_id == target_agent_id)".into(),
        ));
    }

    // 2. Target must exist + be live.
    resolve_target(&state, target_agent_id).await?;

    // 3. Reviewer must exist + be live, and caller must be a member of
    //    the reviewer agent's account.
    let reviewer = sqlx::query!(
        r#"SELECT account_id FROM agents WHERE id = $1 AND tombstoned_at IS NULL"#,
        req.reviewer_agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::InvalidRequest("reviewer_agent_id not found".into()))?;
    if !user_is_member(&state.db, user.user_id, reviewer.account_id).await? {
        return Err(ApiError::Forbidden);
    }

    // 4. Every tag must belong to the target AND the reviewer must
    //    have a non-'rejected' invocation of it. One query verifies
    //    both in a single round-trip.
    //
    //    The query returns one row per *valid* tag; any tag passed in
    //    that doesn't appear in the result set is invalid. We check
    //    set equality rather than just `len == len` so an unknown
    //    capability id is caught regardless of dupes.
    let valid_rows = sqlx::query!(
        r#"
        SELECT c.id
        FROM agent_capabilities c
        WHERE c.id = ANY($1::uuid[])
          AND c.agent_id = $2
          AND EXISTS (
              SELECT 1 FROM relay_invocations i
              WHERE i.grantee_agent_id = $3
                AND i.capability_id = c.id
                AND i.status <> 'rejected'
          )
        "#,
        &req.tagged_capability_ids,
        target_agent_id,
        req.reviewer_agent_id,
    )
    .fetch_all(&state.db)
    .await?;
    let valid_set: std::collections::HashSet<Uuid> = valid_rows.into_iter().map(|r| r.id).collect();
    for tag in &req.tagged_capability_ids {
        if !valid_set.contains(tag) {
            return Err(ApiError::InvalidRequest(format!(
                "cannot tag capability {tag}: either it doesn't belong to the target agent, or you haven't invoked it"
            )));
        }
    }

    // 5. Resolve tier from the relationship state at write-time.
    let tier =
        if have_accepted_friendship(&state.db, req.reviewer_agent_id, target_agent_id).await? {
            "friend"
        } else {
            "public"
        };

    // 6. Upsert the review + atomic tag swap in a single transaction.
    let mut tx = state.db.begin().await?;
    let inserted_id = Uuid::now_v7();
    let row = sqlx::query!(
        r#"
        INSERT INTO agent_reviews
            (id, reviewer_agent_id, target_agent_id, rating, comment, tier)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (reviewer_agent_id, target_agent_id) DO UPDATE
            SET rating = EXCLUDED.rating,
                comment = EXCLUDED.comment,
                tier = EXCLUDED.tier,
                updated_at = now()
        RETURNING id
        "#,
        inserted_id,
        req.reviewer_agent_id,
        target_agent_id,
        req.rating,
        req.comment.as_deref(),
        tier,
    )
    .fetch_one(&mut *tx)
    .await?;
    let review_id = row.id;

    // Tag swap: clear then insert. The transaction makes this look
    // atomic to readers; no half-state where the row has zero tags.
    sqlx::query!(
        r#"DELETE FROM agent_review_tags WHERE review_id = $1"#,
        review_id,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        r#"
        INSERT INTO agent_review_tags (review_id, capability_id)
        SELECT $1, t.id
        FROM unnest($2::uuid[]) AS t(id)
        ON CONFLICT DO NOTHING
        "#,
        review_id,
        &req.tagged_capability_ids,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let dto = fetch_review(&state.db, user.user_id, review_id).await?;
    Ok(Json(dto))
}

// ─── GET /v1/agents/{target}/reviews ─────────────────────

pub async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(target_agent_id): Path<Uuid>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<ReviewListResponse>> {
    resolve_target(&state, target_agent_id).await?;

    let limit = q.limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);

    // `include_hidden=true` is honoured only when the caller is a
    // member of the target's account. For anyone else we silently
    // force-clear it so hidden reviews never leak.
    let target_account: Uuid = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1"#,
        target_agent_id,
    )
    .fetch_one(&state.db)
    .await?;
    let caller_is_member = user_is_member(&state.db, user.user_id, target_account).await?;
    let include_hidden = q.include_hidden.unwrap_or(false) && caller_is_member;

    let tier_filter = match q.tier.as_deref() {
        None | Some("") => None,
        Some(t) if t == "friend" || t == "public" => Some(t.to_string()),
        Some(_) => {
            return Err(ApiError::InvalidRequest(
                "tier must be friend|public".into(),
            ))
        }
    };

    let cursor = match q.cursor.as_deref().map(decode_cursor).transpose() {
        Ok(c) => c,
        Err(_) => return Err(ApiError::InvalidRequest("malformed cursor".into())),
    };
    let (cursor_created_at, cursor_id) = match cursor {
        Some(c) => (Some(c.created_at), Some(c.id)),
        None => (None, None),
    };

    // Fetch limit + 1 so we know whether to set next_cursor.
    let rows = sqlx::query!(
        r#"
        SELECT
            r.id, r.reviewer_agent_id, r.target_agent_id, r.rating, r.comment,
            r.tier, r.hidden_at, r.created_at, r.updated_at,
            ra.slug as r_slug, ra.display_name as r_display,
            racc.id as r_acct_id, racc.slug as r_acct_slug, racc.display_name as r_acct_display,
            ta.slug as t_slug, ta.display_name as t_display,
            tacc.id as t_acct_id, tacc.slug as t_acct_slug, tacc.display_name as t_acct_display,
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.account_id = racc.id AND m.user_id = $5
            ) as "i_authored!",
            COALESCE(
                (
                    SELECT json_agg(json_build_object('capability_id', c.id, 'capability_name', c.name)
                                    ORDER BY c.name)
                    FROM agent_review_tags rt
                    JOIN agent_capabilities c ON c.id = rt.capability_id
                    WHERE rt.review_id = r.id
                ),
                '[]'::json
            ) AS "tags!"
        FROM agent_reviews r
        JOIN agents ra   ON ra.id   = r.reviewer_agent_id
        JOIN accounts racc ON racc.id = ra.account_id
        JOIN agents ta   ON ta.id   = r.target_agent_id
        JOIN accounts tacc ON tacc.id = ta.account_id
        WHERE r.target_agent_id = $1
          AND ($2::boolean OR r.hidden_at IS NULL)
          AND ($3::text    IS NULL OR r.tier = $3::text)
          AND (
              $6::timestamptz IS NULL
              OR (r.created_at, r.id) < ($6::timestamptz, $7::uuid)
          )
        ORDER BY r.created_at DESC, r.id DESC
        LIMIT $4
        "#,
        target_agent_id,
        include_hidden,
        tier_filter,
        limit + 1,
        user.user_id,
        cursor_created_at,
        cursor_id,
    )
    .fetch_all(&state.db)
    .await?;

    let has_more = rows.len() as i64 > limit;
    let page: Vec<_> = rows.into_iter().take(limit as usize).collect();
    let next_cursor = if has_more {
        page.last().map(|r| {
            encode_cursor(&CursorState {
                created_at: r.created_at,
                id: r.id,
            })
        })
    } else {
        None
    };

    let reviews: Vec<ReviewDto> = page
        .into_iter()
        .map(|r| ReviewDto {
            id: r.id,
            reviewer: AgentSummary {
                id: r.reviewer_agent_id,
                slug: r.r_slug,
                display_name: r.r_display,
                account_id: r.r_acct_id,
                account_slug: r.r_acct_slug,
                account_display_name: r.r_acct_display,
            },
            target: AgentSummary {
                id: r.target_agent_id,
                slug: r.t_slug,
                display_name: r.t_display,
                account_id: r.t_acct_id,
                account_slug: r.t_acct_slug,
                account_display_name: r.t_acct_display,
            },
            rating: r.rating,
            comment: r.comment,
            tier: r.tier,
            tags: serde_json::from_value(r.tags).unwrap_or_default(),
            hidden: r.hidden_at.is_some(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            i_authored: r.i_authored,
        })
        .collect();

    // Summary is computed over the same visibility set as the listing
    // (i.e. honours `include_hidden` + `tier` filter). Single query,
    // five FILTER buckets for the distribution.
    let s = sqlx::query!(
        r#"
        SELECT
            AVG(rating)::float8 AS average,
            COUNT(*)::bigint  AS "count!",
            COUNT(*) FILTER (WHERE rating = 1)::bigint AS "one!",
            COUNT(*) FILTER (WHERE rating = 2)::bigint AS "two!",
            COUNT(*) FILTER (WHERE rating = 3)::bigint AS "three!",
            COUNT(*) FILTER (WHERE rating = 4)::bigint AS "four!",
            COUNT(*) FILTER (WHERE rating = 5)::bigint AS "five!"
        FROM agent_reviews
        WHERE target_agent_id = $1
          AND ($2::boolean OR hidden_at IS NULL)
          AND ($3::text    IS NULL OR tier = $3::text)
        "#,
        target_agent_id,
        include_hidden,
        tier_filter,
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(ReviewListResponse {
        reviews,
        next_cursor,
        summary: ReviewSummary {
            average: s.average,
            count: s.count,
            distribution: ReviewDistribution {
                one: s.one,
                two: s.two,
                three: s.three,
                four: s.four,
                five: s.five,
            },
        },
    }))
}

// ─── POST /v1/agents/{target}/reviews/{id}/hide ──────────

pub async fn hide(
    State(state): State<RelayState>,
    user: AuthUser,
    Path((target_agent_id, review_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ReviewDto>> {
    set_hidden(&state, &user, target_agent_id, review_id, true).await
}

pub async fn unhide(
    State(state): State<RelayState>,
    user: AuthUser,
    Path((target_agent_id, review_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ReviewDto>> {
    set_hidden(&state, &user, target_agent_id, review_id, false).await
}

async fn set_hidden(
    state: &RelayState,
    user: &AuthUser,
    target_agent_id: Uuid,
    review_id: Uuid,
    hide_it: bool,
) -> ApiResult<Json<ReviewDto>> {
    // Caller must be a member of the target agent's account. Anyone
    // else gets 403 — including the reviewer who can't hide their own
    // review on someone else's agent.
    let target_account: Uuid = sqlx::query_scalar!(
        r#"SELECT account_id FROM agents WHERE id = $1 AND tombstoned_at IS NULL"#,
        target_agent_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    if !user_is_member(&state.db, user.user_id, target_account).await? {
        return Err(ApiError::Forbidden);
    }

    let updated = sqlx::query!(
        r#"
        UPDATE agent_reviews
           SET hidden_at = CASE WHEN $3::boolean THEN now() ELSE NULL::timestamptz END,
               hidden_by_user_id = CASE WHEN $3::boolean THEN $4::uuid ELSE NULL::uuid END,
               updated_at = now()
         WHERE id = $1 AND target_agent_id = $2
        RETURNING id
        "#,
        review_id,
        target_agent_id,
        hide_it,
        user.user_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(
        fetch_review(&state.db, user.user_id, updated.id).await?,
    ))
}

// ─── GET /v1/agents/{target}/reviews/eligibility ─────────

pub async fn eligibility(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(target_agent_id): Path<Uuid>,
) -> ApiResult<Json<EligibilityResponse>> {
    resolve_target(&state, target_agent_id).await?;

    // Per reviewer-agent the caller owns, list the target's
    // capability ids the reviewer has invoked (status != 'rejected').
    // One query, ARRAY_AGG'd per reviewer.
    let rows = sqlx::query!(
        r#"
        SELECT
            ra.id   AS reviewer_agent_id,
            ra.display_name AS reviewer_display_name,
            COALESCE(
                ARRAY_AGG(DISTINCT c.id) FILTER (WHERE c.id IS NOT NULL),
                ARRAY[]::uuid[]
            ) AS "tagable_capability_ids!"
        FROM agents ra
        JOIN account_memberships m ON m.account_id = ra.account_id
        LEFT JOIN relay_invocations i ON i.grantee_agent_id = ra.id
            AND i.status <> 'rejected'
        LEFT JOIN agent_capabilities c ON c.id = i.capability_id
            AND c.agent_id = $1
        WHERE m.user_id = $2
          AND ra.tombstoned_at IS NULL
          AND ra.id <> $1
        GROUP BY ra.id, ra.display_name
        HAVING COUNT(c.id) > 0
        ORDER BY ra.display_name ASC
        "#,
        target_agent_id,
        user.user_id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(EligibilityResponse {
        eligible: rows
            .into_iter()
            .map(|r| EligibleReviewer {
                reviewer_agent_id: r.reviewer_agent_id,
                reviewer_display_name: r.reviewer_display_name,
                tagable_capability_ids: r.tagable_capability_ids,
            })
            .collect(),
    }))
}

// ─── Internal: fetch a single review ────────────────────

async fn fetch_review(db: &PgPool, user_id: Uuid, review_id: Uuid) -> Result<ReviewDto, ApiError> {
    let r = sqlx::query!(
        r#"
        SELECT
            r.id, r.reviewer_agent_id, r.target_agent_id, r.rating, r.comment,
            r.tier, r.hidden_at, r.created_at, r.updated_at,
            ra.slug as r_slug, ra.display_name as r_display,
            racc.id as r_acct_id, racc.slug as r_acct_slug, racc.display_name as r_acct_display,
            ta.slug as t_slug, ta.display_name as t_display,
            tacc.id as t_acct_id, tacc.slug as t_acct_slug, tacc.display_name as t_acct_display,
            EXISTS(
                SELECT 1 FROM account_memberships m
                WHERE m.account_id = racc.id AND m.user_id = $2
            ) AS "i_authored!",
            COALESCE(
                (
                    SELECT json_agg(json_build_object('capability_id', c.id, 'capability_name', c.name)
                                    ORDER BY c.name)
                    FROM agent_review_tags rt
                    JOIN agent_capabilities c ON c.id = rt.capability_id
                    WHERE rt.review_id = r.id
                ),
                '[]'::json
            ) AS "tags!"
        FROM agent_reviews r
        JOIN agents ra   ON ra.id   = r.reviewer_agent_id
        JOIN accounts racc ON racc.id = ra.account_id
        JOIN agents ta   ON ta.id   = r.target_agent_id
        JOIN accounts tacc ON tacc.id = ta.account_id
        WHERE r.id = $1
        "#,
        review_id,
        user_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(ReviewDto {
        id: r.id,
        reviewer: AgentSummary {
            id: r.reviewer_agent_id,
            slug: r.r_slug,
            display_name: r.r_display,
            account_id: r.r_acct_id,
            account_slug: r.r_acct_slug,
            account_display_name: r.r_acct_display,
        },
        target: AgentSummary {
            id: r.target_agent_id,
            slug: r.t_slug,
            display_name: r.t_display,
            account_id: r.t_acct_id,
            account_slug: r.t_acct_slug,
            account_display_name: r.t_acct_display,
        },
        rating: r.rating,
        comment: r.comment,
        tier: r.tier,
        tags: serde_json::from_value(r.tags).unwrap_or_default(),
        hidden: r.hidden_at.is_some(),
        created_at: r.created_at,
        updated_at: r.updated_at,
        i_authored: r.i_authored,
    })
}

#[cfg(test)]
mod tests {
    //! End-to-end coverage for the reviews handler:
    //! POST/GET/hide/unhide/eligibility, validation rules, tier
    //! stamping, upsert semantics, hide visibility scoping, and
    //! cursor pagination.
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use chakramcp_shared::config::SharedConfig;
    use chakramcp_shared::jwt;
    use http_body_util::BodyExt;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn config() -> SharedConfig {
        SharedConfig {
            database_url: "ignored".into(),
            jwt_secret: "test-secret-test-secret-test-secret-test-secret".into(),
            admin_email: None,
            survey_enabled: false,
            frontend_base_url: "http://localhost:3000".into(),
            app_base_url: "http://localhost:8080".into(),
            relay_base_url: "http://localhost:8090".into(),
            discovery_v2_enabled: false,
            log_filter: "warn".into(),
        }
    }

    fn jwt_for(user_id: Uuid, email: &str) -> String {
        jwt::encode_jwt(
            &jwt::UserClaims::new(user_id, email.to_string(), false, 1),
            "test-secret-test-secret-test-secret-test-secret",
        )
        .unwrap()
    }

    /// Seed a user + account + owner membership and return all three
    /// plus a Bearer JWT for HTTP requests.
    async fn seed_user_acct(pool: &PgPool, prefix: &str) -> (Uuid, Uuid, String) {
        let user_id = Uuid::now_v7();
        let email = format!("{prefix}-{user_id}@t.local");
        sqlx::query!(
            r#"INSERT INTO users (id, email, display_name, password_hash)
               VALUES ($1, $2, 'Test', 'x')"#,
            user_id,
            email,
        )
        .execute(pool)
        .await
        .unwrap();
        let acct_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO accounts (id, slug, display_name, account_type, owner_user_id)
               VALUES ($1, $2, $3, 'individual', $4)"#,
            acct_id,
            format!("{prefix}-{acct_id}"),
            format!("{prefix} Acct"),
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO account_memberships (id, account_id, user_id, role)
               VALUES ($1, $2, $3, 'owner')"#,
            Uuid::now_v7(),
            acct_id,
            user_id,
        )
        .execute(pool)
        .await
        .unwrap();
        (user_id, acct_id, jwt_for(user_id, &email))
    }

    async fn seed_agent(pool: &PgPool, account_id: Uuid, slug: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agents
                  (id, account_id, slug, display_name, description, visibility, mode)
               VALUES ($1, $2, $3, $4, '', 'network', 'pull')"#,
            id,
            account_id,
            slug,
            format!("Agent {slug}"),
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_capability(pool: &PgPool, agent_id: Uuid, name: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO agent_capabilities
                  (id, agent_id, name, description, input_schema, output_schema, visibility)
               VALUES ($1, $2, $3, '', '{}'::jsonb, '{}'::jsonb, 'network')"#,
            id,
            agent_id,
            name,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Insert a relay_invocations row so the reviewer is treated as
    /// having invoked the capability. `status` controls whether it
    /// counts ('succeeded' counts, 'rejected' does not).
    async fn seed_invocation(
        pool: &PgPool,
        granter_agent_id: Uuid,
        grantee_agent_id: Uuid,
        capability_id: Uuid,
        cap_name: &str,
        status: &str,
    ) {
        sqlx::query!(
            r#"INSERT INTO relay_invocations
                  (id, granter_agent_id, grantee_agent_id, capability_id,
                   capability_name, status)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
            Uuid::now_v7(),
            granter_agent_id,
            grantee_agent_id,
            capability_id,
            cap_name,
            status,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert an accepted friendship between two agents. Proposer is
    /// the first argument purely for determinism — the
    /// `have_accepted_friendship` helper checks both directions.
    async fn seed_accepted_friendship(
        pool: &PgPool,
        proposer_user: Uuid,
        proposer_agent: Uuid,
        target_agent: Uuid,
    ) {
        sqlx::query!(
            r#"INSERT INTO friendships
                  (id, proposer_agent_id, target_agent_id, status,
                   proposer_user_id, decided_by_user_id, decided_at)
               VALUES ($1, $2, $3, 'accepted', $4, $4, now())"#,
            Uuid::now_v7(),
            proposer_agent,
            target_agent,
            proposer_user,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn write_review(
        pool: &PgPool,
        token: &str,
        target: Uuid,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/agents/{target}/reviews"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, val)
    }

    async fn list_reviews(
        pool: &PgPool,
        token: &str,
        target: Uuid,
        query: &str,
    ) -> (StatusCode, serde_json::Value) {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{target}/reviews{query}"))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, val)
    }

    async fn post(pool: &PgPool, token: &str, path: &str) -> (StatusCode, serde_json::Value) {
        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let val: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, val)
    }

    // ─── POST happy paths + tier stamping ──────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn write_public_tier_when_no_friendship(pool: PgPool) {
        let (_alice_uid, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;

        let (status, body) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "comment": "loved it",
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        assert!(status.is_success(), "got {status}: {body}");
        assert_eq!(body["rating"], 5);
        assert_eq!(body["tier"], "public");
        assert_eq!(body["comment"], "loved it");
        assert_eq!(body["i_authored"], true);
        assert_eq!(body["tags"].as_array().unwrap().len(), 1);
        assert_eq!(body["tags"][0]["capability_id"], cap.to_string());
        assert_eq!(body["tags"][0]["capability_name"], "translate");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_friend_tier_when_accepted_friendship(pool: PgPool) {
        let (alice_uid, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;
        seed_accepted_friendship(&pool, alice_uid, alice, bob).await;

        let (status, body) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 4,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        assert!(status.is_success(), "got {status}: {body}");
        assert_eq!(body["tier"], "friend");
    }

    // ─── POST validation ────────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn write_rejects_self_review(pool: PgPool) {
        let (_, acct, jwt) = seed_user_acct(&pool, "solo").await;
        let me = seed_agent(&pool, acct, "me-bot").await;
        let cap = seed_capability(&pool, me, "do-thing").await;
        let (status, _) = write_review(
            &pool,
            &jwt,
            me,
            serde_json::json!({
                "reviewer_agent_id": me,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_rejects_out_of_range_rating(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "thing").await;
        seed_invocation(&pool, bob, alice, cap, "thing", "succeeded").await;

        for bad in [0, 6, -1, 99] {
            let (status, _) = write_review(
                &pool,
                &alice_jwt,
                bob,
                serde_json::json!({
                    "reviewer_agent_id": alice,
                    "rating": bad,
                    "tagged_capability_ids": [cap],
                }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "rating {bad} should be rejected"
            );
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_requires_at_least_one_tag(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let (status, _) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_rejects_tag_reviewer_never_invoked(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        // No invocation seeded — tag should be rejected.

        let (status, _) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_rejects_rejected_only_invocations(pool: PgPool) {
        // status='rejected' invocations explicitly should NOT count
        // per the spec ("any non-'rejected' relay_invocations row
        // counts as invoked").
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "rejected").await;

        let (status, _) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_rejects_tag_on_wrong_target_agent(pool: PgPool) {
        // The tag belongs to a third-party agent, not the target.
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let (_, carol_acct, _) = seed_user_acct(&pool, "carol").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let carol = seed_agent(&pool, carol_acct, "carol-bot").await;
        let carol_cap = seed_capability(&pool, carol, "summarize").await;
        seed_invocation(&pool, carol, alice, carol_cap, "summarize", "succeeded").await;

        let (status, _) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [carol_cap],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_rejects_when_caller_doesnt_own_reviewer(pool: PgPool) {
        // alice's jwt + bob's reviewer_agent_id → 403.
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let (_, carol_acct, _) = seed_user_acct(&pool, "carol").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let carol = seed_agent(&pool, carol_acct, "carol-bot").await;
        let cap = seed_capability(&pool, carol, "thing").await;
        seed_invocation(&pool, carol, bob, cap, "thing", "succeeded").await;
        let _ = alice_acct; // unused

        let (status, _) = write_review(
            &pool,
            &alice_jwt,
            carol,
            serde_json::json!({
                "reviewer_agent_id": bob,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn write_is_upsert_one_review_per_pair(pool: PgPool) {
        // Second write should update the existing row, not create a
        // second. Tag set should swap atomically.
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap1 = seed_capability(&pool, bob, "translate").await;
        let cap2 = seed_capability(&pool, bob, "summarize").await;
        seed_invocation(&pool, bob, alice, cap1, "translate", "succeeded").await;
        seed_invocation(&pool, bob, alice, cap2, "summarize", "succeeded").await;

        let (s1, b1) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "comment": "first take",
                "tagged_capability_ids": [cap1],
            }),
        )
        .await;
        assert!(s1.is_success());
        let first_id = b1["id"].as_str().unwrap().to_string();

        let (s2, b2) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 3,
                "comment": "revised",
                "tagged_capability_ids": [cap2],
            }),
        )
        .await;
        assert!(s2.is_success());
        // ON CONFLICT DO UPDATE keeps the original id; only fields swap.
        assert_eq!(b2["id"].as_str().unwrap(), first_id);
        assert_eq!(b2["rating"], 3);
        assert_eq!(b2["comment"], "revised");
        let tags = b2["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 1, "tag set should swap atomically");
        assert_eq!(tags[0]["capability_id"], cap2.to_string());

        // DB invariant: still exactly one review row for this pair.
        let count: i64 = sqlx::query_scalar!(
            r#"SELECT COUNT(*)::bigint as "n!" FROM agent_reviews
                WHERE reviewer_agent_id = $1 AND target_agent_id = $2"#,
            alice,
            bob,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    // ─── GET list + summary ─────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn list_returns_summary_and_distribution(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _bob_jwt) = seed_user_acct(&pool, "bob").await;
        let (_, carol_acct, carol_jwt) = seed_user_acct(&pool, "carol").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let carol = seed_agent(&pool, carol_acct, "carol-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;
        seed_invocation(&pool, bob, carol, cap, "translate", "succeeded").await;

        write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        write_review(
            &pool,
            &carol_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": carol,
                "rating": 3,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;

        let (status, body) = list_reviews(&pool, &alice_jwt, bob, "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["reviews"].as_array().unwrap().len(), 2);
        assert_eq!(body["summary"]["count"], 2);
        assert_eq!(body["summary"]["average"].as_f64().unwrap(), 4.0);
        assert_eq!(body["summary"]["distribution"]["5"], 1);
        assert_eq!(body["summary"]["distribution"]["3"], 1);
        assert_eq!(body["summary"]["distribution"]["1"], 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn list_filters_by_tier(pool: PgPool) {
        let (alice_uid, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let (_, carol_acct, carol_jwt) = seed_user_acct(&pool, "carol").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let carol = seed_agent(&pool, carol_acct, "carol-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;
        seed_invocation(&pool, bob, carol, cap, "translate", "succeeded").await;
        // Alice ↔ Bob are friends; Carol is not.
        seed_accepted_friendship(&pool, alice_uid, alice, bob).await;

        write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        write_review(
            &pool,
            &carol_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": carol,
                "rating": 2,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;

        let (_, friend_only) = list_reviews(&pool, &alice_jwt, bob, "?tier=friend").await;
        assert_eq!(friend_only["reviews"].as_array().unwrap().len(), 1);
        assert_eq!(friend_only["reviews"][0]["tier"], "friend");
        let (_, public_only) = list_reviews(&pool, &alice_jwt, bob, "?tier=public").await;
        assert_eq!(public_only["reviews"].as_array().unwrap().len(), 1);
        assert_eq!(public_only["reviews"][0]["tier"], "public");
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn list_invalid_tier_returns_400(pool: PgPool) {
        let (_, acct, jwt) = seed_user_acct(&pool, "alice").await;
        let bob = seed_agent(&pool, acct, "bob-bot").await;
        let (status, _) = list_reviews(&pool, &jwt, bob, "?tier=garbage").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ─── Hide/unhide ─────────────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn hide_then_unhide_round_trip(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, bob_jwt) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;

        let (_, body) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 2,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        let review_id = body["id"].as_str().unwrap();

        // Bob (target owner) can hide.
        let (status, hidden_body) = post(
            &pool,
            &bob_jwt,
            &format!("/v1/agents/{bob}/reviews/{review_id}/hide"),
        )
        .await;
        assert!(status.is_success(), "hide returned {status}: {hidden_body}");
        assert_eq!(hidden_body["hidden"], true);

        // Anonymous-to-bob list (alice's jwt): hidden reviews are filtered out.
        let (_, list_body) = list_reviews(&pool, &alice_jwt, bob, "").await;
        assert_eq!(list_body["reviews"].as_array().unwrap().len(), 0);
        assert_eq!(list_body["summary"]["count"], 0);

        // Member of target with include_hidden=true sees it.
        let (_, list_for_owner) = list_reviews(&pool, &bob_jwt, bob, "?include_hidden=true").await;
        assert_eq!(list_for_owner["reviews"].as_array().unwrap().len(), 1);
        assert_eq!(list_for_owner["reviews"][0]["hidden"], true);

        // Bob unhides.
        let (status, unhidden) = post(
            &pool,
            &bob_jwt,
            &format!("/v1/agents/{bob}/reviews/{review_id}/unhide"),
        )
        .await;
        assert!(status.is_success(), "unhide returned {status}: {unhidden}");
        assert_eq!(unhidden["hidden"], false);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn hide_forbidden_for_non_target_member(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;
        let (_, body) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 1,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        let review_id = body["id"].as_str().unwrap();
        // Alice (the reviewer, not target owner) tries to hide → 403.
        let (status, _) = post(
            &pool,
            &alice_jwt,
            &format!("/v1/agents/{bob}/reviews/{review_id}/hide"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn list_include_hidden_silently_dropped_for_non_member(pool: PgPool) {
        // Even with `include_hidden=true`, non-members must not see
        // hidden reviews. The handler silently force-clears the flag.
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, bob_jwt) = seed_user_acct(&pool, "bob").await;
        let alice = seed_agent(&pool, alice_acct, "alice-bot").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice, cap, "translate", "succeeded").await;
        let (_, body) = write_review(
            &pool,
            &alice_jwt,
            bob,
            serde_json::json!({
                "reviewer_agent_id": alice,
                "rating": 5,
                "tagged_capability_ids": [cap],
            }),
        )
        .await;
        let review_id = body["id"].as_str().unwrap();
        post(
            &pool,
            &bob_jwt,
            &format!("/v1/agents/{bob}/reviews/{review_id}/hide"),
        )
        .await;

        let (_, list_body) = list_reviews(&pool, &alice_jwt, bob, "?include_hidden=true").await;
        assert_eq!(list_body["reviews"].as_array().unwrap().len(), 0);
    }

    // ─── Eligibility ─────────────────────────────────────────

    #[sqlx::test(migrations = "../migrations")]
    async fn eligibility_lists_caller_agents_with_invoked_caps(pool: PgPool) {
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice1 = seed_agent(&pool, alice_acct, "alice1").await;
        let alice2 = seed_agent(&pool, alice_acct, "alice2").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap_a = seed_capability(&pool, bob, "translate").await;
        let cap_b = seed_capability(&pool, bob, "summarize").await;
        // alice1 invoked translate. alice2 invoked summarize.
        seed_invocation(&pool, bob, alice1, cap_a, "translate", "succeeded").await;
        seed_invocation(&pool, bob, alice2, cap_b, "summarize", "succeeded").await;

        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{bob}/reviews/eligibility"))
                    .header(header::AUTHORIZATION, format!("Bearer {alice_jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let eligible = body["eligible"].as_array().unwrap();
        assert_eq!(eligible.len(), 2);
        // Sorted by display_name ASC.
        let names: Vec<&str> = eligible
            .iter()
            .map(|e| e["reviewer_display_name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Agent alice1", "Agent alice2"]);
        // alice1 has cap_a in its tagable_capability_ids, alice2 has cap_b.
        let a1 = &eligible[0];
        assert_eq!(a1["reviewer_agent_id"], alice1.to_string());
        let a1_caps = a1["tagable_capability_ids"].as_array().unwrap();
        assert_eq!(a1_caps.len(), 1);
        assert_eq!(a1_caps[0], cap_a.to_string());
        let a2 = &eligible[1];
        let a2_caps = a2["tagable_capability_ids"].as_array().unwrap();
        assert_eq!(a2_caps[0], cap_b.to_string());
    }

    #[sqlx::test(migrations = "../migrations")]
    async fn eligibility_excludes_agents_with_no_invoked_caps(pool: PgPool) {
        // alice has two agents but only one has invoked any capability
        // of the target. The other should NOT appear.
        let (_, alice_acct, alice_jwt) = seed_user_acct(&pool, "alice").await;
        let (_, bob_acct, _) = seed_user_acct(&pool, "bob").await;
        let alice_active = seed_agent(&pool, alice_acct, "alice-active").await;
        let _alice_idle = seed_agent(&pool, alice_acct, "alice-idle").await;
        let bob = seed_agent(&pool, bob_acct, "bob-bot").await;
        let cap = seed_capability(&pool, bob, "translate").await;
        seed_invocation(&pool, bob, alice_active, cap, "translate", "succeeded").await;

        let app = crate::router(crate::state::RelayState::new(pool.clone(), config()));
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/v1/agents/{bob}/reviews/eligibility"))
                    .header(header::AUTHORIZATION, format!("Bearer {alice_jwt}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let eligible = body["eligible"].as_array().unwrap();
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0]["reviewer_agent_id"], alice_active.to_string());
    }
}
