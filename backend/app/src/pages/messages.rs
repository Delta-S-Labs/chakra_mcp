//! `/app/messages/{id}`: one of an agent's approvers answers a call to its
//! `human_in_loop` capability themselves, on a self-hosted server (waking
//! pull agents §9; chakramcp.com's web app has its own page at the same
//! path). Agents pass this link (`owner_reply_url`) to their owner, whether
//! or not they claimed the call. Answering goes through
//! `chakramcp_shared::consent::reply::answer_as_owner`, the same code as an
//! approver's result posted from the web app.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::Form;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use chakramcp_shared::consent::decide::Actor;
use chakramcp_shared::consent::owner_reply::{self, MessageView, ReplyShape};
use chakramcp_shared::consent::reply::{self, Draft};
use chakramcp_shared::error::ApiError;

use super::approvals::{input_fields, when};
use super::security::{message, render, AppOrigin};
use super::session::{self, PageUser};
use super::views::{ReplyPage, SigninPage};
use super::{csrf, expired_form, server_error, signin_failure};
use crate::accounts;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ReplyInput {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    step: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    reply: String,
    #[serde(default)]
    reason: String,
}

fn back_here(id: Uuid) -> String {
    format!("/app/messages/{id}")
}

fn no_such_message(origin: &AppOrigin) -> Response {
    message(
        origin,
        StatusCode::NOT_FOUND,
        "Message not found",
        "No message has that link, or it's for an agent you don't answer for.",
        None,
    )
}

/// `GET /app/messages/{id}`
pub(crate) async fn show(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Response {
    let origin = AppOrigin::of(&state);
    let Ok(id) = Uuid::parse_str(&id) else {
        return no_such_message(&origin);
    };
    let (jar, token) = csrf::token(jar, &origin);
    let page = match session::current(&state, &jar).await {
        None => signin(&state, &origin, &token, id, "", None),
        Some(user) => reply_page(&state, &origin, &user, &token, id, None, None).await,
    };
    (jar, page).into_response()
}

/// `POST /app/messages/{id}`: the sign-in form, and reply or decline.
pub(crate) async fn submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(input): Form<ReplyInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !csrf::verify(&jar, &headers, &input.csrf, &origin) {
        return expired_form(&origin);
    }
    let Ok(id) = Uuid::parse_str(&id) else {
        return no_such_message(&origin);
    };
    if input.step == "signin" {
        return match accounts::authenticate(&state.db, &input.email, &input.password).await {
            Ok(user) => match session::start(&state, jar, &user, &origin) {
                Ok(jar) => (jar, Redirect::to(&back_here(id))).into_response(),
                Err(e) => server_error(&origin, e),
            },
            Err(e) => signin(
                &state,
                &origin,
                &input.csrf,
                id,
                &input.email,
                Some(signin_failure(e)),
            ),
        };
    }
    let Some(user) = session::current(&state, &jar).await else {
        return signin(
            &state,
            &origin,
            &input.csrf,
            id,
            "",
            Some((
                StatusCode::UNAUTHORIZED,
                "Your sign-in expired. Sign in again.".into(),
            )),
        );
    };
    let view = match owner_reply::load(&state.db, user.user_id, id).await {
        Ok(view) => view,
        Err(ApiError::NotFound) => return no_such_message(&origin),
        Err(e) => return server_error(&origin, e),
    };
    let draft = match input.step.as_str() {
        "reply" => match ReplyShape::of(&view.output_schema).output(&input.reply, Utc::now()) {
            Ok(output) => Draft {
                status: "succeeded".into(),
                output: Some(output),
                error: None,
            },
            Err(problem) => {
                return reply_page(
                    &state,
                    &origin,
                    &user,
                    &input.csrf,
                    id,
                    Some(&input),
                    Some(problem),
                )
                .await
            }
        },
        "decline" => {
            // Message safety (when on) screens the reason with the reply.
            let reason = input.reason.trim();
            Draft {
                status: "failed".into(),
                output: None,
                error: Some(if reason.is_empty() {
                    "The owner declined to answer.".into()
                } else {
                    reason.to_owned()
                }),
            }
        }
        _ => {
            return message(
                &origin,
                StatusCode::BAD_REQUEST,
                "Something went wrong",
                "That form didn't say what to do. Go back and try again.",
                None,
            )
        }
    };
    let declined = draft.status == "failed";
    let actor = Actor {
        user_id: user.user_id,
        minted_jti: None,
    };
    match reply::answer_as_owner(
        &state.db,
        &state.wake,
        state.system_one.as_deref(),
        actor,
        id,
        draft,
    )
    .await
    {
        Ok(_) if declined => message(
            &origin,
            StatusCode::OK,
            "Declined",
            "The caller is told, with your reason if you gave one.",
            None,
        ),
        Ok(_) => message(
            &origin,
            StatusCode::OK,
            "Sent",
            "Your reply is on its way to the caller.",
            None,
        ),
        Err(ApiError::NotFound | ApiError::Forbidden) => no_such_message(&origin),
        Err(e @ (ApiError::Database(_) | ApiError::Internal(_))) => server_error(&origin, e),
        // Settled meanwhile, blocked by message safety, or too large: show
        // the call as it now stands, with what went wrong.
        Err(problem) => {
            reply_page(
                &state,
                &origin,
                &user,
                &input.csrf,
                id,
                Some(&input),
                Some(problem.to_string()),
            )
            .await
        }
    }
}

fn signin(
    state: &AppState,
    origin: &AppOrigin,
    csrf: &str,
    id: Uuid,
    email: &str,
    failure: Option<(StatusCode, String)>,
) -> Response {
    let signup_href = state
        .hosting
        .signup_enabled
        .then(|| format!("/signup?return_to={}", urlencoding::encode(&back_here(id))));
    let (status, error) = match failure {
        Some((status, error)) => (status, Some(error)),
        None => (StatusCode::OK, None),
    };
    render(
        status,
        &SigninPage {
            title: "Sign in".into(),
            host: origin.host(),
            lead: "to answer a message to your agent".into(),
            action: back_here(id),
            csrf: csrf.to_owned(),
            hidden: vec![],
            email: email.to_owned(),
            error,
            signup_href,
        },
        &[],
    )
}

/// How a call that can't be answered here ended, in a sentence.
fn outcome_text(view: &MessageView) -> String {
    match view.status.as_str() {
        "succeeded" => "It's been answered.".into(),
        "failed" => format!(
            "It was declined{}.",
            view.error_message
                .as_deref()
                .map(|e| format!(": {e}"))
                .unwrap_or_default()
        ),
        "timeout" => "Nobody answered it in time, so it timed out.".into(),
        "rejected" => "It was refused.".into(),
        "awaiting_consent" => "It's waiting for your approval first.".into(),
        other => format!("It's {other}."),
    }
}

async fn reply_page(
    state: &AppState,
    origin: &AppOrigin,
    user: &PageUser,
    csrf: &str,
    id: Uuid,
    form: Option<&ReplyInput>,
    error: Option<String>,
) -> Response {
    let view = match owner_reply::load(&state.db, user.user_id, id).await {
        Ok(view) => view,
        Err(ApiError::NotFound) => return no_such_message(origin),
        Err(e) => return server_error(origin, e),
    };
    let answerable = view.answerable();
    let heading = if answerable {
        "Answer a message?"
    } else {
        "A message to your agent"
    };
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    render(
        status,
        &ReplyPage {
            title: heading.into(),
            host: origin.host(),
            heading: heading.into(),
            action: back_here(id),
            csrf: csrf.to_owned(),
            user_email: user.email.clone(),
            requester: view.requester.clone().unwrap_or_else(|| "An agent".into()),
            requester_account: view.requester_account.clone(),
            verified: view.requester_verified,
            capability: view.capability_name.clone(),
            capability_description: view.capability_description.clone(),
            target: view.target.clone().unwrap_or_default(),
            created_at: when(view.created_at),
            expires_at: view.expires_at.map(when),
            input: input_fields(view.input.as_ref()),
            answerable,
            json: ReplyShape::of(&view.output_schema) == ReplyShape::Json,
            reply: form.map(|f| f.reply.clone()).unwrap_or_default(),
            reason: form.map(|f| f.reason.clone()).unwrap_or_default(),
            error,
            draft_link: view
                .draft_request_id
                .filter(|_| matches!(view.status.as_str(), "pending" | "in_progress"))
                .map(|request| format!("/app/approvals/{request}")),
            outcome: outcome_text(&view),
        },
        &[],
    )
}
