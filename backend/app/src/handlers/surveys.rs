//! First-login survey handlers.
//!
//! Two endpoints, both `JWT`-auth:
//!   GET  /v1/me/survey   → existing answers (or null if not yet)
//!   POST /v1/me/survey   → upsert answers
//!
//! `is_survey_required(state, user_id)` is also called from
//! `handlers::users::me` so the frontend knows whether to redirect.

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::error::{ApiError, ApiResult};

use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SurveyDto {
    pub use_case: Option<String>,
    pub agent_types: Vec<String>,
    pub frameworks: Vec<String>,
    pub scale: Option<String>,
    pub notes: Option<String>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitSurveyRequest {
    pub use_case: Option<String>,
    pub agent_types: Vec<String>,
    pub frameworks: Vec<String>,
    pub scale: Option<String>,
    pub notes: Option<String>,
}

// ─────────────────────────────────────────────────────────
// GET /v1/me/survey
// ─────────────────────────────────────────────────────────
pub async fn get_mine(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Option<SurveyDto>>> {
    let row = fetch_survey(&state.db, user.user_id).await?;
    Ok(Json(row))
}

// ─────────────────────────────────────────────────────────
// POST /v1/me/survey  (upsert)
// ─────────────────────────────────────────────────────────
pub async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<SubmitSurveyRequest>,
) -> ApiResult<Json<SurveyDto>> {
    if let Some(s) = req.scale.as_deref() {
        if !matches!(s, "exploring" | "team" | "company" | "production") {
            return Err(ApiError::InvalidRequest(
                "scale must be exploring|team|company|production".into(),
            ));
        }
    }
    if req.agent_types.iter().any(|s| s.is_empty()) || req.frameworks.iter().any(|s| s.is_empty()) {
        return Err(ApiError::InvalidRequest(
            "agent_types and frameworks must not contain empty strings".into(),
        ));
    }

    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO surveys (id, user_id, use_case, agent_types, frameworks, scale, notes)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (user_id) DO UPDATE SET
            use_case    = EXCLUDED.use_case,
            agent_types = EXCLUDED.agent_types,
            frameworks  = EXCLUDED.frameworks,
            scale       = EXCLUDED.scale,
            notes       = EXCLUDED.notes,
            completed_at = now()
        "#,
        id,
        user.user_id,
        req.use_case,
        &req.agent_types,
        &req.frameworks,
        req.scale,
        req.notes,
    )
    .execute(&state.db)
    .await?;

    let saved = fetch_survey(&state.db, user.user_id)
        .await?
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("survey vanished after upsert")))?;
    Ok(Json(saved))
}

/// Decide whether the survey overlay should be required for this user.
/// Returns true only when SURVEY_ENABLED=true AND the user has no
/// completed survey yet.
pub async fn is_required(
    db: &PgPool,
    survey_enabled: bool,
    user_id: Uuid,
) -> Result<bool, ApiError> {
    if !survey_enabled {
        return Ok(false);
    }
    let row = sqlx::query!(
        r#"SELECT 1 as one FROM surveys WHERE user_id = $1 LIMIT 1"#,
        user_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.is_none())
}

async fn fetch_survey(db: &PgPool, user_id: Uuid) -> Result<Option<SurveyDto>, ApiError> {
    let row = sqlx::query!(
        r#"
        SELECT use_case, agent_types, frameworks, scale, notes, completed_at
        FROM surveys
        WHERE user_id = $1
        LIMIT 1
        "#,
        user_id
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(|r| SurveyDto {
        use_case: r.use_case,
        agent_types: r.agent_types,
        frameworks: r.frameworks,
        scale: r.scale,
        notes: r.notes,
        completed_at: r.completed_at,
    }))
}
