//! `/app/approvals/{id}`: approve or deny a request held for the owner's
//! consent, on a self-hosted server (owner consent spec §7; chakramcp.com's
//! web app has its own page at the same path). The agent passes this link
//! to its owner. Deciding goes through `chakramcp_shared::consent::decide`,
//! the same code as the relay's approvals API.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::Form;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use chakramcp_shared::consent::decide::{self, Actor, ApprovalView, DecideError, Narrowing};
use chakramcp_shared::consent::terms::{narrowing_options, ConsentTerms, Validity};

use super::security::{message, render, AppOrigin};
use super::session::{self, PageUser};
use super::views::{ApprovalPage, Choice, Field, SigninPage};
use super::{csrf, expired_form, server_error, signin_failure};
use crate::accounts;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ApprovalInput {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    step: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    valid_for: String,
    #[serde(default)]
    max_requests: String,
    #[serde(default)]
    note: String,
    /// For a reply: the hash of the draft the approver was shown.
    #[serde(default)]
    draft_sha256: String,
}

fn back_here(id: Uuid) -> String {
    format!("/app/approvals/{id}")
}

fn no_such_request(origin: &AppOrigin) -> Response {
    message(
        origin,
        StatusCode::NOT_FOUND,
        "Request not found",
        "No request has that link, or it's for an agent you can't see.",
        None,
    )
}

/// `GET /app/approvals/{id}`
pub(crate) async fn show(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Response {
    let origin = AppOrigin::of(&state);
    let Ok(id) = Uuid::parse_str(&id) else {
        return no_such_request(&origin);
    };
    let (jar, token) = csrf::token(jar, &origin);
    let page = match session::current(&state, &jar).await {
        None => signin(&state, &origin, &token, id, "", None),
        Some(user) => approval_page(&state, &origin, &user, &token, id, None, None).await,
    };
    (jar, page).into_response()
}

/// `POST /app/approvals/{id}`: the sign-in form, and approve or deny.
pub(crate) async fn submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(input): Form<ApprovalInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !csrf::verify(&jar, &headers, &input.csrf, &origin) {
        return expired_form(&origin);
    }
    let Ok(id) = Uuid::parse_str(&id) else {
        return no_such_request(&origin);
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
    let actor = Actor {
        user_id: user.user_id,
        minted_jti: None,
    };
    let outcome = match input.step.as_str() {
        "approve" => match narrowing(&input) {
            Ok(narrowing) => {
                let is_reply = narrowing.draft_sha256.is_some();
                decide::approve(&state.db, &state.wake, actor, id, narrowing)
                    .await
                    .map(|_| {
                        if is_reply {
                            ("Sent", "Your agent's reply is on its way to the requester.")
                        } else {
                            ("Approved", "Your agent receives the request now.")
                        }
                    })
            }
            Err(problem) => Err(DecideError::BadRequest(problem)),
        },
        "deny" => {
            // Message safety (when on): the note goes to the requester.
            let note = chakramcp_shared::system_one::note_or_refuse(
                state.system_one.as_deref(),
                Some(input.note.clone()),
                chakramcp_shared::system_one::NoteAbout {
                    kind: "deny_note",
                    ..Default::default()
                },
            )
            .await;
            match note {
                Err(refused) => {
                    return message(
                        &origin,
                        StatusCode::FORBIDDEN,
                        "Your note wasn't sent",
                        &format!("{refused}. Change the note and try again."),
                        None,
                    )
                }
                Ok(note) => decide::deny(&state.db, &state.wake, actor, id, note.as_deref())
                    .await
                    .map(|_| {
                        (
                            "Declined",
                            "The requester is told, with your note if you left one.",
                        )
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
    match outcome {
        Ok((heading, text)) => message(&origin, StatusCode::OK, heading, text, None),
        Err(DecideError::NotFound) => no_such_request(&origin),
        Err(DecideError::NoLongerAuthorized(reason)) => message(
            &origin,
            StatusCode::CONFLICT,
            "Can't approve this any more",
            &format!("No longer authorized: {reason}. The request was cancelled."),
            None,
        ),
        Err(DecideError::Db(e)) => server_error(&origin, e),
        Err(DecideError::Internal(e)) => server_error(&origin, e),
        Err(DecideError::Forbidden) => {
            approval_page(
                &state,
                &origin,
                &user,
                &input.csrf,
                id,
                Some(&input),
                Some("Only this capability's approvers can decide on it.".into()),
            )
            .await
        }
        // Not an approver, already decided, expired or bad terms: show the
        // request again, as it now stands, with what went wrong.
        Err(problem) => {
            approval_page(
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

/// The approver's choices: a validity no longer than asked, and a request
/// limit. Blank fields keep what was asked.
fn narrowing(input: &ApprovalInput) -> Result<Narrowing, String> {
    let valid_for = match input.valid_for.trim() {
        "" => None,
        raw => Some(Validity::parse(raw)?),
    };
    let max_requests = match input.max_requests.trim() {
        "" => None,
        raw => Some(
            raw.parse::<i32>()
                .map_err(|_| "The request limit must be a whole number.".to_owned())?,
        ),
    };
    Ok(Narrowing {
        valid_for,
        max_requests,
        draft_sha256: Some(input.draft_sha256.clone()).filter(|h| !h.is_empty()),
    })
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
            lead: "to decide on a request to your agent".into(),
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

pub(super) fn when(at: DateTime<Utc>) -> String {
    at.format("%-d %b %Y, %H:%M UTC").to_string()
}

/// The input's top-level fields, as text: strings as they are, anything
/// else as JSON. A non-object input is one field.
pub(super) fn input_fields(input: Option<&Value>) -> Vec<Field> {
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match input {
        None | Some(Value::Null) => vec![],
        Some(Value::Object(map)) => map
            .iter()
            .map(|(name, value)| Field {
                name: name.clone(),
                value: text(value),
            })
            .collect(),
        Some(other) => vec![Field {
            name: "input".into(),
            value: text(other),
        }],
    }
}

/// "For 1 day" → "for 1 day", to follow a colon mid-sentence.
fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What happened to a decided request, in a sentence.
fn outcome_text(view: &ApprovalView, expired: bool) -> String {
    let at = view.decided_at.map(when).unwrap_or_default();
    let note = view
        .decision_note
        .as_deref()
        .map(|n| format!(" Note: {n}"))
        .unwrap_or_default();
    if view.kind == "reply" {
        return match view.status.as_str() {
            "approved" => format!("Confirmed and sent on {at}."),
            "denied" => format!("Rejected on {at}.{note}"),
            _ if expired || view.status == "expired" => {
                "Nobody confirmed this reply in time, so the call failed.".into()
            }
            other => format!("This reply is {other}."),
        };
    }
    match view.status.as_str() {
        "approved" => match view
            .approved_terms
            .clone()
            .and_then(|t| serde_json::from_value::<ConsentTerms>(t).ok())
        {
            Some(terms) => format!("Approved on {at}: {}.", lowercase_first(&terms.describe())),
            None => format!("Approved on {at}."),
        },
        "denied" => format!("Denied on {at}.{note}"),
        "cancelled" => format!("Cancelled on {at}.{note}"),
        _ if expired || view.status == "expired" => {
            "This request expired before anyone decided.".into()
        }
        other => format!("This request is {other}."),
    }
}

/// The request as it stands, with the form if the user may decide. `form`
/// keeps what the user chose when the page is shown again with an error.
async fn approval_page(
    state: &AppState,
    origin: &AppOrigin,
    user: &PageUser,
    csrf: &str,
    id: Uuid,
    form: Option<&ApprovalInput>,
    error: Option<String>,
) -> Response {
    let view = match decide::load_view(&state.db, user.user_id, id).await {
        Ok(view) => view,
        Err(DecideError::NotFound) => return no_such_request(origin),
        Err(e) => return server_error(origin, e),
    };
    let requested: ConsentTerms = serde_json::from_value(view.terms.clone()).unwrap_or_default();
    let max_validity = Validity::parse(&view.max_validity).unwrap_or(Validity::Once);
    let chosen = form
        .map(|f| f.valid_for.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| requested.valid_for.to_string());
    let validity: Vec<Choice> = narrowing_options(requested.valid_for, max_validity.secs() as i32)
        .into_iter()
        .map(|v| Choice {
            value: v.to_string(),
            label: v.label(),
            checked: v.to_string() == chosen,
        })
        .collect();
    let expired = view.status == "pending" && view.expires_at <= Utc::now();
    let pending = view.status == "pending" && !expired;
    let is_reply = view.kind == "reply";
    let heading = match (is_reply, pending) {
        (true, true) => "Confirm a reply?",
        (true, false) => "A reply from your agent",
        (false, true) => "Approve a request?",
        (false, false) => "A request to your agent",
    };
    let draft: Option<chakramcp_shared::consent::reply::Draft> = view
        .draft
        .clone()
        .and_then(|d| serde_json::from_value(d).ok());
    let draft_error = draft
        .as_ref()
        .filter(|d| d.status == "failed")
        .map(|d| d.error.clone().unwrap_or_else(|| "it failed".into()));
    let approvers = view
        .approvers
        .iter()
        .map(|a| a.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    render(
        status,
        &ApprovalPage {
            title: heading.into(),
            host: origin.host(),
            heading: heading.into(),
            action: back_here(id),
            csrf: csrf.to_owned(),
            user_email: user.email.clone(),
            requester: view
                .requester
                .agent_display_name
                .clone()
                .unwrap_or_else(|| "An agent".into()),
            requester_account: view.requester.account_display_name.clone(),
            verified: view.requester.verified,
            capability: view
                .capability
                .name
                .clone()
                .unwrap_or_else(|| "a deleted capability".into()),
            capability_description: view
                .capability
                .description
                .clone()
                .filter(|d| !d.trim().is_empty()),
            target: view.target.display_name.clone().unwrap_or_default(),
            purpose: requested.purpose.clone(),
            grant_purpose: view.grant_purpose.clone(),
            asked: requested.describe(),
            expires_at: when(view.expires_at),
            created_at: when(view.created_at),
            input: input_fields(view.input.as_ref()),
            pending,
            may_decide: view.may_decide,
            approvers: if approvers.is_empty() {
                "the account's owners and admins".into()
            } else {
                approvers
            },
            outcome: outcome_text(&view, expired),
            validity,
            standing_allowed: requested.is_standing(),
            asked_max: requested.max_requests,
            max_requests: form.map(|f| f.max_requests.clone()).unwrap_or_else(|| {
                requested
                    .max_requests
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            }),
            note: form.map(|f| f.note.clone()).unwrap_or_default(),
            error,
            is_reply,
            draft: draft
                .as_ref()
                .filter(|d| d.status != "failed")
                .map(|d| input_fields(d.output.as_ref()))
                .unwrap_or_default(),
            draft_error,
            draft_sha256: view.draft_sha256.clone().unwrap_or_default(),
        },
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_fields_show_top_level_values() {
        let fields = input_fields(Some(&json!({"date": "2026-10-24", "slots": [1, 2]})));
        let pairs: Vec<(String, String)> = fields.into_iter().map(|f| (f.name, f.value)).collect();
        assert_eq!(
            pairs,
            [
                ("date".to_owned(), "2026-10-24".to_owned()),
                ("slots".to_owned(), "[1,2]".to_owned())
            ]
        );
        assert_eq!(input_fields(Some(&json!("hi")))[0].value, "hi");
        assert!(input_fields(None).is_empty());
    }

    #[test]
    fn blank_choices_keep_what_was_asked() {
        let n = narrowing(&ApprovalInput::default()).unwrap();
        assert!(n.valid_for.is_none() && n.max_requests.is_none());
        let input = ApprovalInput {
            valid_for: "1d".into(),
            max_requests: "3".into(),
            ..ApprovalInput::default()
        };
        let n = narrowing(&input).unwrap();
        assert_eq!(n.valid_for, Some(Validity::Secs(86400)));
        assert_eq!(n.max_requests, Some(3));
        let bad = ApprovalInput {
            max_requests: "lots".into(),
            ..ApprovalInput::default()
        };
        assert!(narrowing(&bad).is_err());
    }
}
