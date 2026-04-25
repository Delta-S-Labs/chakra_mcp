use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use chakramcp_shared::error::ApiResult;

use crate::state::RelayState;

pub async fn healthz() -> Json<Value> {
    Json(json!({ "ok": true, "service": "chakramcp-relay" }))
}

pub async fn readyz(State(state): State<RelayState>) -> ApiResult<Json<Value>> {
    sqlx::query!("SELECT 1 as one").fetch_one(&state.db).await?;
    Ok(Json(json!({ "ok": true, "db": "up" })))
}
