//! Calls routes (spec `2026-10-25-calls-design.md` §11). Merged into the
//! router only when `CALLS_ENABLED` is on; off, they 404. The work is in
//! `crate::calls`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chakramcp_shared::error::ApiResult;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::calls::{feed, lifecycle, messages, read};
use crate::state::RelayState;

pub fn routes() -> Router<RelayState> {
    Router::new()
        .route("/v1/calls", post(invite).get(list))
        .route("/v1/calls/{id}", get(get_one))
        .route("/v1/calls/{id}/accept", post(accept))
        .route("/v1/calls/{id}/decline", post(decline))
        .route("/v1/calls/{id}/end", post(end))
        .route("/v1/calls/{id}/owner-end", post(owner_end))
        .route("/v1/calls/{id}/content", delete(delete_content))
        .route("/v1/calls/{id}/messages", post(send).get(history))
        .route("/v1/inbox/calls", get(inbox))
}

async fn invite(
    State(state): State<RelayState>,
    user: AuthUser,
    Json(req): Json<lifecycle::InviteRequest>,
) -> ApiResult<impl IntoResponse> {
    let call = lifecycle::invite(&state, &user, req).await?;
    Ok((StatusCode::CREATED, Json(call.dto())))
}

async fn list(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<read::ListQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(read::list(&state, &user, q).await?))
}

async fn get_one(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    Ok(Json(read::get(&state, &user, id).await?))
}

async fn accept(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<lifecycle::AcceptRequest>,
) -> ApiResult<Json<Value>> {
    Ok(Json(lifecycle::accept(&state, &user, id, req).await?.dto()))
}

async fn decline(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<lifecycle::DeclineRequest>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        lifecycle::decline(&state, &user, id, req).await?.dto(),
    ))
}

async fn end(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<lifecycle::EndRequest>,
) -> ApiResult<Json<Value>> {
    Ok(Json(lifecycle::end(&state, &user, id, req).await?.dto()))
}

async fn owner_end(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    Ok(Json(lifecycle::owner_end(&state, &user, id).await?.dto()))
}

async fn delete_content(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    lifecycle::delete_content(&state, &user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn send(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<messages::SendRequest>,
) -> ApiResult<impl IntoResponse> {
    let entry = messages::send(&state, &user, id, req).await?;
    Ok((StatusCode::CREATED, Json(entry.dto())))
}

async fn history(
    State(state): State<RelayState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Query(q): Query<messages::HistoryQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    Ok(Json(messages::history(&state, &user, id, q).await?))
}

async fn inbox(
    State(state): State<RelayState>,
    user: AuthUser,
    Query(q): Query<feed::FeedQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    Ok(Json(feed::claim(&state, &user, q).await?))
}
