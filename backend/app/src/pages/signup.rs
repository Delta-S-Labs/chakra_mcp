//! `/signup`, while sign-up is open (`SIGNUP_ENABLED`). Creates the account
//! through the same function as `POST /v1/auth/signup`; nobody becomes an
//! admin here.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::{Form, Query};
use serde::Deserialize;

use chakramcp_shared::error::ApiError;

use super::security::{message, render, AppOrigin};
use super::views::SignupPage;
use super::{csrf, expired_form, server_error, session};
use crate::accounts::{self, NewUser, SignedIn};
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct SignupInput {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    return_to: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    password: String,
}

fn closed(origin: &AppOrigin) -> Response {
    message(
        origin,
        StatusCode::NOT_FOUND,
        "Sign-up is closed",
        "This server doesn't take sign-ups. Ask whoever runs it to create your account.",
        None,
    )
}

fn page(
    origin: &AppOrigin,
    csrf: &str,
    return_to: &str,
    input: Option<&SignupInput>,
    error: Option<String>,
) -> Response {
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    render(
        status,
        &SignupPage {
            title: "Create an account".into(),
            host: origin.host(),
            csrf: csrf.to_owned(),
            return_to: return_to.to_owned(),
            name: input.map(|i| i.name.clone()).unwrap_or_default(),
            email: input.map(|i| i.email.clone()).unwrap_or_default(),
            error,
            signin_href: (!return_to.is_empty()).then(|| return_to.to_owned()),
        },
        &[],
    )
}

/// `GET /signup`
pub(crate) async fn show(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(input): Query<SignupInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !state.hosting.signup_enabled {
        return closed(&origin);
    }
    let (jar, token) = csrf::token(jar, &origin);
    let return_to = origin.return_path(&input.return_to).unwrap_or_default();
    (jar, page(&origin, &token, &return_to, None, None)).into_response()
}

/// `POST /signup`
pub(crate) async fn submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(input): Form<SignupInput>,
) -> Response {
    let origin = AppOrigin::of(&state);
    if !csrf::verify(&jar, &headers, &input.csrf, &origin) {
        return expired_form(&origin);
    }
    if !state.hosting.signup_enabled {
        return closed(&origin);
    }
    let return_to = origin.return_path(&input.return_to);
    let created = accounts::create_user(
        &state.db,
        NewUser {
            email: &input.email,
            name: &input.name,
            password: &input.password,
            is_admin: false,
        },
    )
    .await;
    let created = match created {
        Ok(created) => created,
        Err(ApiError::InvalidRequest(problem) | ApiError::Conflict(problem)) => {
            let return_to = return_to.unwrap_or_default();
            return page(
                &origin,
                &input.csrf,
                &return_to,
                Some(&input),
                Some(problem),
            );
        }
        Err(e) => return server_error(&origin, e),
    };
    let user = SignedIn {
        user_id: created.user_id,
        email: created.email,
        display_name: created.display_name,
        avatar_url: created.avatar_url,
        is_admin: false,
    };
    let jar = match session::start(&state, jar, &user, &origin) {
        Ok(jar) => jar,
        Err(e) => return server_error(&origin, e),
    };
    match return_to {
        Some(path) => (jar, Redirect::to(&path)).into_response(),
        None => (
            jar,
            message(
                &origin,
                StatusCode::OK,
                "Account created",
                "Next, sign in from the command line with `chakramcp login`.",
                None,
            ),
        )
            .into_response(),
    }
}
