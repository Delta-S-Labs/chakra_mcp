//! `/oauth/authorize`: sign in, then consent. `chakramcp login` and MCP
//! clients send the browser here (the discovery document's
//! `authorization_endpoint`); approving mints the code through the same
//! function as `POST /oauth/issue-code`.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::{Form, Query};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use chakramcp_shared::error::ApiError;

use super::security::{message, origin_of, render, AppOrigin};
use super::session::{self, PageUser};
use super::views::{AgentOption, ConsentPage, Hidden, SigninPage};
use super::{csrf, expired_form, server_error, signin_failure};
use crate::accounts;
use crate::handlers::oauth::{self, ClientPreview, IssueCodeRequest};
use crate::state::AppState;

/// The authorization request (query on GET, form on POST) and the form's own
/// fields.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AuthorizeInput {
    #[serde(default)]
    pub response_type: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub code_challenge: String,
    #[serde(default)]
    pub code_challenge_method: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub scope: String,
    /// On the request, the client's hint; on the consent form, the choice.
    #[serde(default)]
    pub agent_scope: String,
    /// The client's hint: agent ids, comma-separated.
    #[serde(default)]
    pub agent_ids: String,
    #[serde(default)]
    pub csrf: String,
    #[serde(default)]
    pub step: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub password: String,
    /// Agents ticked on the consent form.
    #[serde(default)]
    pub selected: Vec<String>,
}

/// A request that passed the checks.
struct Request {
    client: ClientPreview,
    redirect_uri: String,
    code_challenge: String,
    state: String,
    agent_scope_hint: String,
    agent_ids_hint: String,
}

impl Request {
    /// The request as hidden form fields. The consent form leaves out the
    /// agent-scope hint: there the field is the user's choice.
    fn hidden(&self, with_scope_hint: bool) -> Vec<Hidden> {
        let mut fields = vec![
            Hidden {
                name: "response_type",
                value: "code".into(),
            },
            Hidden {
                name: "client_id",
                value: self.client.client_id.clone(),
            },
            Hidden {
                name: "redirect_uri",
                value: self.redirect_uri.clone(),
            },
            Hidden {
                name: "code_challenge",
                value: self.code_challenge.clone(),
            },
            Hidden {
                name: "code_challenge_method",
                value: "S256".into(),
            },
            Hidden {
                name: "state",
                value: self.state.clone(),
            },
            Hidden {
                name: "scope",
                value: "relay.full".into(),
            },
            Hidden {
                name: "agent_ids",
                value: self.agent_ids_hint.clone(),
            },
        ];
        if with_scope_hint {
            fields.push(Hidden {
                name: "agent_scope",
                value: self.agent_scope_hint.clone(),
            });
        }
        fields
    }

    /// This request's own page: where sign-in and sign-up come back to.
    fn path(&self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for field in self.hidden(true) {
            if !field.value.is_empty() {
                query.append_pair(field.name, &field.value);
            }
        }
        format!("/oauth/authorize?{}", query.finish())
    }
}

/// Check a request in the order RFC 6749 §4.1.2.1 sets: without a known
/// client and one of its registered redirect URIs, show an error page and
/// never redirect; past that, report problems to the client. The error is
/// the page or redirect to send back.
#[allow(clippy::result_large_err)]
async fn check(
    state: &AppState,
    origin: &AppOrigin,
    input: &AuthorizeInput,
) -> Result<Request, Response> {
    let client = match oauth::load_client(&state.db, &input.client_id).await {
        Ok(client) => client,
        Err(ApiError::NotFound) => {
            return Err(message(
                origin,
                StatusCode::BAD_REQUEST,
                "Unknown application",
                "This sign-in link names an application this server doesn't know. Start again from the application.",
                None,
            ))
        }
        Err(e) => return Err(server_error(origin, e)),
    };
    if !client.redirect_uris.contains(&input.redirect_uri) {
        return Err(message(
            origin,
            StatusCode::BAD_REQUEST,
            "Unknown return address",
            "This sign-in link would send you somewhere the application didn't register. Start again from the application.",
            None,
        ));
    }
    let to_client = |error: &str, description: &str| {
        Err(redirect(
            &input.redirect_uri,
            &[("error", error), ("error_description", description)],
            &input.state,
        ))
    };
    if input.response_type != "code" {
        return to_client(
            "unsupported_response_type",
            "only response_type=code is supported",
        );
    }
    if !(input.code_challenge_method.is_empty() || input.code_challenge_method == "S256") {
        return to_client("invalid_request", "code_challenge_method must be S256");
    }
    if !(43..=128).contains(&input.code_challenge.len()) {
        return to_client(
            "invalid_request",
            "a PKCE code_challenge (S256) of 43-128 characters is required",
        );
    }
    if !(input.scope.is_empty() || input.scope == "relay.full") {
        return to_client("invalid_scope", "only scope=relay.full is supported");
    }
    Ok(Request {
        client,
        redirect_uri: input.redirect_uri.clone(),
        code_challenge: input.code_challenge.clone(),
        state: input.state.clone(),
        agent_scope_hint: input.agent_scope.clone(),
        agent_ids_hint: input.agent_ids.clone(),
    })
}

/// `303` to the client's redirect URI with these parameters, plus `state`
/// when the request had one.
fn redirect(uri: &str, params: &[(&str, &str)], state: &str) -> Response {
    let Ok(mut url) = Url::parse(uri) else {
        return (StatusCode::BAD_REQUEST, "invalid redirect_uri").into_response();
    };
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in params {
            query.append_pair(key, value);
        }
        if !state.is_empty() {
            query.append_pair("state", state);
        }
    }
    Redirect::to(url.as_str()).into_response()
}

/// `GET /oauth/authorize`
pub(crate) async fn show(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(input): Query<AuthorizeInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    let request = match check(&state, &origin, &input).await {
        Ok(request) => request,
        Err(page) => return page,
    };
    let (jar, token) = csrf::token(jar, &origin);
    let page = match session::current(&state, &jar).await {
        Some(user) => consent(&state, &origin, &request, &user, &token, None, None).await,
        None => signin(&state, &origin, &request, &token, "", None),
    };
    (jar, page).into_response()
}

/// `POST /oauth/authorize`: the sign-in and consent forms.
pub(crate) async fn submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(input): Form<AuthorizeInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !csrf::verify(&jar, &headers, &input.csrf, &origin) {
        return expired_form(&origin);
    }
    let request = match check(&state, &origin, &input).await {
        Ok(request) => request,
        Err(page) => return page,
    };
    match input.step.as_str() {
        "signin" => match accounts::authenticate(&state.db, &input.email, &input.password).await {
            Ok(user) => match session::start(&state, jar, &user, &origin) {
                Ok(jar) => (jar, Redirect::to(&request.path())).into_response(),
                Err(e) => server_error(&origin, e),
            },
            Err(e) => signin(
                &state,
                &origin,
                &request,
                &input.csrf,
                &input.email,
                Some(signin_failure(e)),
            ),
        },
        "approve" => {
            let Some(user) = session::current(&state, &jar).await else {
                return signin(
                    &state,
                    &origin,
                    &request,
                    &input.csrf,
                    "",
                    Some((
                        StatusCode::UNAUTHORIZED,
                        "Your sign-in expired. Sign in again.".into(),
                    )),
                );
            };
            let scope = match input.agent_scope.as_str() {
                "own" | "selected" => input.agent_scope.clone(),
                _ => "all".to_owned(),
            };
            let selected: Vec<Uuid> = input
                .selected
                .iter()
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect();
            let choice = Some((scope.as_str(), input.selected.as_slice()));
            if scope == "selected" && selected.is_empty() {
                return consent(
                    &state,
                    &origin,
                    &request,
                    &user,
                    &input.csrf,
                    choice,
                    Some("Pick at least one agent, or choose another option.".into()),
                )
                .await;
            }
            let code_request = IssueCodeRequest {
                client_id: request.client.client_id.clone(),
                redirect_uri: request.redirect_uri.clone(),
                code_challenge: request.code_challenge.clone(),
                code_challenge_method: "S256".into(),
                scope: "relay.full".into(),
                agent_scope: Some(scope.clone()),
                selected_agent_ids: (scope == "selected").then_some(selected),
            };
            match oauth::issue_code_for(&state.db, user.user_id, &code_request).await {
                Ok(issued) => redirect(
                    &request.redirect_uri,
                    &[("code", &issued.code)],
                    &request.state,
                ),
                Err(ApiError::InvalidRequest(problem)) => {
                    consent(
                        &state,
                        &origin,
                        &request,
                        &user,
                        &input.csrf,
                        choice,
                        Some(problem),
                    )
                    .await
                }
                Err(e) => server_error(&origin, e),
            }
        }
        "deny" => redirect(
            &request.redirect_uri,
            &[
                ("error", "access_denied"),
                ("error_description", "the user denied access"),
            ],
            &request.state,
        ),
        _ => message(
            &origin,
            StatusCode::BAD_REQUEST,
            "Something went wrong",
            "That form didn't say what to do. Go back and try again.",
            None,
        ),
    }
}

fn signin(
    state: &AppState,
    origin: &AppOrigin,
    request: &Request,
    csrf: &str,
    email: &str,
    failure: Option<(StatusCode, String)>,
) -> Response {
    let signup_href = state
        .hosting
        .signup_enabled
        .then(|| format!("/signup?return_to={}", urlencoding::encode(&request.path())));
    let (status, error) = match failure {
        Some((status, error)) => (status, Some(error)),
        None => (StatusCode::OK, None),
    };
    render(
        status,
        &SigninPage {
            title: "Sign in".into(),
            host: origin.host(),
            lead: format!("to continue to {}", request.client.client_name),
            action: "/oauth/authorize",
            csrf: csrf.to_owned(),
            hidden: request.hidden(true),
            email: email.to_owned(),
            error,
            signup_href,
        },
        &[],
    )
}

/// The consent page. `choice` keeps what the user picked when the page is
/// shown again with an error.
async fn consent(
    state: &AppState,
    origin: &AppOrigin,
    request: &Request,
    user: &PageUser,
    csrf: &str,
    choice: Option<(&str, &[String])>,
    error: Option<String>,
) -> Response {
    let agents = match oauth::agents_for_user(&state.db, user.user_id).await {
        Ok(agents) => agents,
        Err(e) => return server_error(origin, e),
    };
    let (scope, ticked): (&str, Vec<String>) = match choice {
        Some((scope, ticked)) => (scope, ticked.to_vec()),
        None => (
            request.agent_scope_hint.as_str(),
            request
                .agent_ids_hint
                .split(',')
                .map(|id| id.trim().to_owned())
                .collect(),
        ),
    };
    let agent_scope = match scope {
        "own" => "own",
        "selected" if !agents.is_empty() => "selected",
        _ => "all",
    };
    let agents = agents
        .into_iter()
        .map(|agent| {
            let id = agent.id.to_string();
            AgentOption {
                checked: ticked.contains(&id),
                id,
                slug: agent.slug,
                display_name: agent.display_name,
                account_slug: agent.account_slug,
            }
        })
        .collect();
    let redirect_host = Url::parse(&request.redirect_uri)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_owned(),
            })
        })
        .unwrap_or_else(|| request.redirect_uri.clone());
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    let form_action: Vec<String> = origin_of(&request.redirect_uri).into_iter().collect();
    render(
        status,
        &ConsentPage {
            title: format!("Allow {}", request.client.client_name),
            host: origin.host(),
            client_name: request.client.client_name.clone(),
            client_uri: request.client.client_uri.clone(),
            redirect_host,
            user_email: user.email.clone(),
            csrf: csrf.to_owned(),
            hidden: request.hidden(false),
            agent_scope: agent_scope.to_owned(),
            agents,
            error,
            return_to: request.path(),
        },
        &form_action,
    )
}
