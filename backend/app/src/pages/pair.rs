//! `/app/pair`: approve a device pairing (RFC 8628). `chakramcp pair`,
//! `chakramcp login --method device` and agents that pair themselves send
//! people here (the device flow's `verification_uri`); approving and denying
//! go through the same functions as `POST /oauth/device-approve` and
//! `/oauth/device-deny`.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::{Form, Query};
use serde::Deserialize;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::security::{message, render, AppOrigin};
use super::session::{self, PageUser};
use super::views::{AgentOption, Hidden, PairCodePage, PairPage, SigninPage};
use super::{csrf, expired_form, server_error, signin_failure};
use crate::accounts;
use crate::handlers::oauth::{self, DeviceApproveRequest};
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PairInput {
    /// The pairing code (the web UI's parameter name).
    #[serde(default)]
    session: String,
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    step: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    /// An existing agent's id, or `new`.
    #[serde(default)]
    target: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    visibility: String,
}

/// `abcd efgh` or `ABCDEFGH` → `ABCD-EFGH`.
fn normalize_code(raw: &str) -> String {
    let code: String = raw
        .trim()
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if code.len() == 8 && code.chars().all(|c| c.is_ascii_alphanumeric()) {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code
    }
}

/// A slug from an agent's name: lowercase letters, digits and single hyphens.
fn slug_from(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').chars().take(32).collect()
}

fn back_here(code: &str) -> String {
    if code.is_empty() {
        "/app/pair".to_owned()
    } else {
        format!("/app/pair?session={}", urlencoding::encode(code))
    }
}

/// `GET /app/pair`
pub(crate) async fn show(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(input): Query<PairInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    let (jar, token) = csrf::token(jar, &origin);
    let code = normalize_code(&input.session);
    let page = match session::current(&state, &jar).await {
        None => signin(&state, &origin, &token, &code, "", None),
        Some(_) if code.is_empty() => code_page(&origin, "", None),
        Some(user) => request_page(&state, &origin, &user, &token, &code, None, None).await,
    };
    (jar, page).into_response()
}

/// `POST /app/pair`: the sign-in and approval forms.
pub(crate) async fn submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(input): Form<PairInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !csrf::verify(&jar, &headers, &input.csrf, &origin) {
        return expired_form(&origin);
    }
    let code = normalize_code(&input.session);
    if input.step == "signin" {
        return match accounts::authenticate(&state.db, &input.email, &input.password).await {
            Ok(user) => match session::start(&state, jar, &user, &origin) {
                Ok(jar) => (jar, Redirect::to(&back_here(&code))).into_response(),
                Err(e) => server_error(&origin, e),
            },
            Err(e) => signin(
                &state,
                &origin,
                &input.csrf,
                &code,
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
            &code,
            "",
            Some((
                StatusCode::UNAUTHORIZED,
                "Your sign-in expired. Sign in again.".into(),
            )),
        );
    };
    match input.step.as_str() {
        "approve" => {
            let request = if input.target.is_empty() || input.target == "new" {
                DeviceApproveRequest {
                    user_code: code.clone(),
                    existing_agent_id: None,
                    agent_slug: Some(input.slug.clone()),
                    agent_display_name: Some(input.display_name.clone()),
                    agent_description: Some(input.description.trim().to_owned())
                        .filter(|d| !d.is_empty()),
                    agent_visibility: Some(input.visibility.clone()).filter(|v| !v.is_empty()),
                    account_slug: None,
                    agent_scope: None,
                    selected_agent_ids: None,
                }
            } else {
                let Ok(agent_id) = Uuid::parse_str(&input.target) else {
                    return request_page(
                        &state,
                        &origin,
                        &user,
                        &input.csrf,
                        &code,
                        Some(&input),
                        Some("Pick one of your agents, or a new one.".into()),
                    )
                    .await;
                };
                DeviceApproveRequest {
                    user_code: code.clone(),
                    existing_agent_id: Some(agent_id),
                    agent_slug: None,
                    agent_display_name: None,
                    agent_description: None,
                    agent_visibility: None,
                    account_slug: None,
                    agent_scope: None,
                    selected_agent_ids: None,
                }
            };
            match oauth::approve_device(&state.db, user.user_id, &request).await {
                Ok(done) => message(
                    &origin,
                    StatusCode::OK,
                    "Approved",
                    &format!(
                        "{}/{} is connected. The device finishes on its own within a few seconds, so you can close this page.",
                        done.account_slug, done.agent_slug
                    ),
                    None,
                ),
                Err(ApiError::InvalidRequest(problem) | ApiError::Conflict(problem)) => {
                    request_page(
                        &state,
                        &origin,
                        &user,
                        &input.csrf,
                        &code,
                        Some(&input),
                        Some(problem),
                    )
                    .await
                }
                Err(ApiError::NotFound) => code_page(&origin, &code, Some(unknown_code())),
                Err(e) => server_error(&origin, e),
            }
        }
        "deny" => match oauth::deny_device(&state.db, &code).await {
            Ok(()) => message(
                &origin,
                StatusCode::OK,
                "Denied",
                "The device won't be connected.",
                None,
            ),
            Err(ApiError::Conflict(problem)) => message(
                &origin,
                StatusCode::CONFLICT,
                "Too late to deny",
                &problem,
                None,
            ),
            Err(ApiError::NotFound) => code_page(&origin, &code, Some(unknown_code())),
            Err(e) => server_error(&origin, e),
        },
        _ => message(
            &origin,
            StatusCode::BAD_REQUEST,
            "Something went wrong",
            "That form didn't say what to do. Go back and try again.",
            None,
        ),
    }
}

fn unknown_code() -> String {
    "No pairing request has that code. Check it and try again.".to_owned()
}

fn signin(
    state: &AppState,
    origin: &AppOrigin,
    csrf: &str,
    code: &str,
    email: &str,
    failure: Option<(StatusCode, String)>,
) -> Response {
    let signup_href = state.hosting.signup_enabled.then(|| {
        format!(
            "/signup?return_to={}",
            urlencoding::encode(&back_here(code))
        )
    });
    let (status, error) = match failure {
        Some((status, error)) => (status, Some(error)),
        None => (StatusCode::OK, None),
    };
    render(
        status,
        &SigninPage {
            title: "Sign in".into(),
            host: origin.host(),
            lead: "to approve a device pairing".into(),
            action: "/app/pair",
            csrf: csrf.to_owned(),
            hidden: vec![Hidden {
                name: "session",
                value: code.to_owned(),
            }],
            email: email.to_owned(),
            error,
            signup_href,
        },
        &[],
    )
}

fn code_page(origin: &AppOrigin, code: &str, error: Option<String>) -> Response {
    let status = if error.is_some() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::OK
    };
    render(
        status,
        &PairCodePage {
            title: "Connect an agent".into(),
            host: origin.host(),
            code: code.to_owned(),
            error,
        },
        &[],
    )
}

/// The approval form for a pending request, or where the request stands.
/// `form` keeps what the user entered when the page is shown again with an
/// error.
async fn request_page(
    state: &AppState,
    origin: &AppOrigin,
    user: &PageUser,
    csrf: &str,
    code: &str,
    form: Option<&PairInput>,
    error: Option<String>,
) -> Response {
    let info = match oauth::device_session_info(&state.db, code).await {
        Ok(info) => info,
        Err(ApiError::NotFound) => return code_page(origin, code, Some(unknown_code())),
        Err(e) => return server_error(origin, e),
    };
    match info.status {
        "pending" => {}
        "approved" | "consumed" => {
            return message(
                origin,
                StatusCode::OK,
                "Already approved",
                "This pairing request was approved. The device finishes on its own.",
                None,
            )
        }
        "denied" => {
            return message(
                origin,
                StatusCode::OK,
                "Denied",
                "This pairing request was denied.",
                None,
            )
        }
        _ => {
            return message(
                origin,
                StatusCode::GONE,
                "This code expired",
                "Pairing codes last 10 minutes. Start pairing again on the device.",
                None,
            )
        }
    }
    let agents = match oauth::agents_for_user(&state.db, user.user_id).await {
        Ok(agents) => agents,
        Err(e) => return server_error(origin, e),
    };
    let target = form.map(|f| f.target.clone()).unwrap_or_default();
    let display_name = form
        .map(|f| f.display_name.clone())
        .or_else(|| info.agent_display_name_hint.clone())
        .unwrap_or_default();
    let slug = form
        .map(|f| f.slug.clone())
        .or_else(|| info.agent_slug_hint.clone())
        .unwrap_or_else(|| slug_from(&display_name));
    let description = form
        .map(|f| f.description.clone())
        .or_else(|| info.agent_description_hint.clone())
        .unwrap_or_default();
    let visibility = form
        .map(|f| f.visibility.clone())
        .filter(|v| !v.is_empty())
        .or_else(|| info.agent_visibility_hint.clone())
        .unwrap_or_else(|| "private".into());
    let agents: Vec<AgentOption> = agents
        .into_iter()
        .map(|agent| {
            let id = agent.id.to_string();
            AgentOption {
                checked: target == id,
                id,
                slug: agent.slug,
                display_name: agent.display_name,
                account_slug: agent.account_slug,
            }
        })
        .collect();
    let new_agent = !agents.iter().any(|a| a.checked);
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    render(
        status,
        &PairPage {
            title: "Connect an agent".into(),
            host: origin.host(),
            user_email: user.email.clone(),
            user_code: code.to_owned(),
            persona: info.persona.clone(),
            csrf: csrf.to_owned(),
            agents,
            new_agent,
            display_name,
            slug,
            description,
            visibility,
            error,
        },
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::{normalize_code, slug_from};

    #[test]
    fn codes_are_normalized() {
        assert_eq!(normalize_code("abcd-efgh"), "ABCD-EFGH");
        assert_eq!(normalize_code(" abcdefgh "), "ABCD-EFGH");
        assert_eq!(normalize_code("ABCD EFGH"), "ABCD-EFGH");
        assert_eq!(normalize_code(""), "");
    }

    #[test]
    fn slugs_come_from_names() {
        assert_eq!(slug_from("Release Notes Bot"), "release-notes-bot");
        assert_eq!(slug_from("  Ünïcode -- agent!! "), "n-code-agent");
        assert_eq!(slug_from(&"x".repeat(40)).len(), 32);
    }
}
