//! The pages, driven as a browser would: cookies kept between requests,
//! forms posted with their hidden fields and an `Origin` header.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use chakramcp_shared::hosting::{HostingMode, HostingSettings};

use crate::accounts::{self, NewUser};
use crate::tests_support::test_config;
use crate::AppState;

/// `test_config`'s app URL.
const ORIGIN: &str = "http://localhost:8080";
const CLIENT: &str = "mcp_test";
const REDIRECT: &str = "http://127.0.0.1:9999/callback";
const VERIFIER: &str = "verifier-0123456789012345678901234567890123456789";
const PASSWORD: &str = "pages-test-password";

fn challenge() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

fn authorize_path() -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT)
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("code_challenge", &challenge())
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", "xyz");
    format!("/oauth/authorize?{}", query.finish())
}

struct Page {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Page {
    fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }

    /// The page's hidden form fields, unescaped. Fields repeated across the
    /// page's forms (`csrf`) keep their first value.
    fn hidden(&self) -> Vec<(String, String)> {
        let mut fields = Vec::new();
        for chunk in self.body.split("<input type=\"hidden\" name=\"").skip(1) {
            let Some((name, rest)) = chunk.split_once('"') else {
                continue;
            };
            let Some(rest) = rest.strip_prefix(" value=\"") else {
                continue;
            };
            let Some((value, _)) = rest.split_once('"') else {
                continue;
            };
            if !fields.iter().any(|(n, _): &(String, String)| n == name) {
                fields.push((name.to_owned(), unescape(value)));
            }
        }
        fields
    }

    fn field(&self, name: &str) -> String {
        self.hidden()
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }
}

/// Decode HTML character references, as a browser does for attribute
/// values: `&amp;`, `&quot;`, `&lt;`, `&gt;` and numeric ones (`&#38;`,
/// `&#x27;`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A browser: keeps cookies, sends `Origin` on posts.
struct Browser {
    app: axum::Router,
    cookies: HashMap<String, String>,
    origin: Option<String>,
}

impl Browser {
    fn new(state: AppState) -> Self {
        Self {
            app: crate::router(state),
            cookies: HashMap::new(),
            origin: Some(ORIGIN.into()),
        }
    }

    async fn get(&mut self, path: &str) -> Page {
        self.send(Method::GET, path, None).await
    }

    async fn post(&mut self, path: &str, form: &[(String, String)]) -> Page {
        self.send(Method::POST, path, Some(form)).await
    }

    async fn send(
        &mut self,
        method: Method,
        path: &str,
        form: Option<&[(String, String)]>,
    ) -> Page {
        let mut request = Request::builder().method(method).uri(path);
        if !self.cookies.is_empty() {
            let cookies: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookies.join("; "));
        }
        let body = match form {
            Some(pairs) => {
                request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                if let Some(origin) = &self.origin {
                    request = request.header(header::ORIGIN, origin);
                }
                let mut encoded = url::form_urlencoded::Serializer::new(String::new());
                for (key, value) in pairs {
                    encoded.append_pair(key, value);
                }
                Body::from(encoded.finish())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        for cookie in response.headers().get_all(header::SET_COOKIE) {
            let cookie = cookie.to_str().unwrap();
            let (pair, attributes) = cookie.split_once(';').unwrap_or((cookie, ""));
            let (name, value) = pair.split_once('=').unwrap();
            if value.is_empty() || attributes.contains("Max-Age=0") {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Page {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// Submit the page's form: its hidden fields plus `extra`.
    async fn submit(&mut self, path: &str, page: &Page, extra: &[(&str, &str)]) -> Page {
        let mut form = page.hidden();
        for (name, value) in extra {
            form.retain(|(n, _)| n != name);
            form.push(((*name).to_owned(), (*value).to_owned()));
        }
        self.post(path, &form).await
    }
}

fn state(pool: &PgPool) -> AppState {
    AppState::new(pool.clone(), test_config())
}

async fn seed(pool: &PgPool) -> Uuid {
    sqlx::query(
        "INSERT INTO oauth_clients (id, client_id, client_name, redirect_uris)
         VALUES ($1, $2, 'Test Client', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(CLIENT)
    .bind(vec![REDIRECT.to_owned()])
    .execute(pool)
    .await
    .unwrap();
    accounts::create_user(
        pool,
        NewUser {
            email: "ada@example.test",
            name: "Ada",
            password: PASSWORD,
            is_admin: false,
        },
    )
    .await
    .unwrap()
    .user_id
}

/// Sign in through the authorize page; returns the consent page.
async fn sign_in(browser: &mut Browser) -> Page {
    let signin = browser.get(&authorize_path()).await;
    assert_eq!(signin.status, StatusCode::OK, "{}", signin.body);
    assert!(signin.body.contains("Sign in"));
    let done = browser
        .submit(
            "/oauth/authorize",
            &signin,
            &[("email", "ada@example.test"), ("password", PASSWORD)],
        )
        .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.body);
    let back = done.location().to_owned();
    assert!(back.starts_with("/oauth/authorize?"), "{back}");
    let consent = browser.get(&back).await;
    assert_eq!(consent.status, StatusCode::OK, "{}", consent.body);
    assert!(consent
        .body
        .contains("Test Client wants to use your account"));
    consent
}

async fn redeem(state: AppState, code: &str) -> (StatusCode, Value) {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "authorization_code")
        .append_pair("code", code)
        .append_pair("client_id", CLIENT)
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("code_verifier", VERIFIER);
    let response = crate::router(state)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form.finish()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn get_with_bearer(state: AppState, path: &str, token: &str) -> StatusCode {
    crate::router(state)
        .oneshot(
            Request::builder()
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

fn query_param(location: &str, name: &str) -> Option<String> {
    let url = url::Url::parse(location).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

#[sqlx::test(migrations = "../migrations")]
async fn sign_in_consent_and_the_code_redeems(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    let consent = sign_in(&mut browser).await;

    // Strict headers, and the approve step may redirect to the client.
    let csp = consent.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(
        csp.contains("form-action 'self' http://127.0.0.1:9999"),
        "{csp}"
    );
    assert_eq!(consent.headers[header::REFERRER_POLICY], "same-origin");
    assert_eq!(consent.headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(consent.headers[header::CACHE_CONTROL], "no-store");

    let approved = browser
        .submit(
            "/oauth/authorize",
            &consent,
            &[("step", "approve"), ("agent_scope", "all")],
        )
        .await;
    assert_eq!(approved.status, StatusCode::SEE_OTHER, "{}", approved.body);
    let location = approved.location();
    assert!(location.starts_with(REDIRECT), "{location}");
    assert_eq!(query_param(location, "state").as_deref(), Some("xyz"));
    let code = query_param(location, "code").expect("a code");

    let (status, token) = redeem(state(&pool), &code).await;
    assert_eq!(status, StatusCode::OK, "{token}");
    let access = token["access_token"].as_str().unwrap();
    assert_eq!(
        get_with_bearer(state(&pool), "/v1/me", access).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn the_session_cookie_is_not_an_api_token(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    sign_in(&mut browser).await;
    let session = browser.cookies["chakramcp_page_session"].clone();
    assert_eq!(
        get_with_bearer(state(&pool), "/v1/me", &session).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn forms_from_elsewhere_are_refused(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    let signin = browser.get(&authorize_path()).await;
    let wrong_password = [("email", "ada@example.test"), ("password", "not-it-at-all")];

    // No token, a wrong token, a foreign Origin, `Origin: null`.
    let mut without_token = signin.hidden();
    without_token.retain(|(n, _)| n != "csrf");
    without_token.extend(
        wrong_password
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into())),
    );
    assert_eq!(
        browser
            .post("/oauth/authorize", &without_token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    let wrong = browser
        .submit(
            "/oauth/authorize",
            &signin,
            &[("csrf", "x".repeat(43).as_str())],
        )
        .await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);

    for origin in ["https://evil.example", "null"] {
        browser.origin = Some(origin.into());
        let page = browser
            .submit("/oauth/authorize", &signin, &wrong_password)
            .await;
        assert_eq!(page.status, StatusCode::FORBIDDEN, "Origin: {origin}");
    }

    // No Origin at all (curl): the token decides, and this one is right.
    browser.origin = None;
    let page = browser
        .submit("/oauth/authorize", &signin, &wrong_password)
        .await;
    assert_eq!(page.status, StatusCode::UNAUTHORIZED, "{}", page.body);
    assert!(page.body.contains("Wrong email or password"));
}

#[sqlx::test(migrations = "../migrations")]
async fn deny_sends_access_denied(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    let consent = sign_in(&mut browser).await;
    let denied = browser
        .submit("/oauth/authorize", &consent, &[("step", "deny")])
        .await;
    assert_eq!(denied.status, StatusCode::SEE_OTHER);
    assert_eq!(
        query_param(denied.location(), "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(
        query_param(denied.location(), "state").as_deref(),
        Some("xyz")
    );
    assert!(query_param(denied.location(), "code").is_none());
}

#[sqlx::test(migrations = "../migrations")]
async fn bad_requests_never_redirect_to_an_unregistered_address(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    let unknown_client = authorize_path().replace(CLIENT, "mcp_nope");
    let page = browser.get(&unknown_client).await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(page.location().is_empty());
    assert!(page.body.contains("Unknown application"));

    let elsewhere = authorize_path().replace("127.0.0.1%3A9999", "evil.example");
    let page = browser.get(&elsewhere).await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(page.location().is_empty(), "{}", page.location());

    // Past those checks, problems go back to the client.
    let no_pkce = authorize_path().replace(&challenge(), "short");
    let page = browser.get(&no_pkce).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert_eq!(
        query_param(page.location(), "error").as_deref(),
        Some("invalid_request")
    );
}

#[sqlx::test(migrations = "../migrations")]
async fn a_selected_agent_scope_reaches_the_token(pool: PgPool) {
    let user = seed(&pool).await;
    let account: Uuid =
        sqlx::query_scalar("SELECT account_id FROM account_memberships WHERE user_id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
    let agent = crate::tests_support::seed_agent(&pool, account, "helper", user).await;

    let mut browser = Browser::new(state(&pool));
    let consent = sign_in(&mut browser).await;
    assert!(
        consent.body.contains("helper"),
        "the user's agents are offered"
    );

    let none_ticked = browser
        .submit(
            "/oauth/authorize",
            &consent,
            &[("step", "approve"), ("agent_scope", "selected")],
        )
        .await;
    assert_eq!(none_ticked.status, StatusCode::BAD_REQUEST);
    assert!(none_ticked.body.contains("Pick at least one agent"));

    let approved = browser
        .submit(
            "/oauth/authorize",
            &consent,
            &[
                ("step", "approve"),
                ("agent_scope", "selected"),
                ("selected", &agent.to_string()),
            ],
        )
        .await;
    let code = query_param(approved.location(), "code").expect("a code");
    let (status, _) = redeem(state(&pool), &code).await;
    assert_eq!(status, StatusCode::OK);
    let scoped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM credential_scope_agents a
           JOIN credential_scopes s ON s.id = a.credential_scope_id
          WHERE s.agent_scope = 'selected' AND a.agent_id = $1",
    )
    .bind(agent)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(scoped, 1);
}

#[sqlx::test(migrations = "../migrations")]
async fn sign_up_follows_the_setting(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    assert_eq!(browser.get("/signup").await.status, StatusCode::NOT_FOUND);
    let signin = browser.get(&authorize_path()).await;
    assert!(
        !signin.body.contains("/signup"),
        "no sign-up link while closed"
    );

    let open = state(&pool).with_hosting(HostingSettings {
        signup_enabled: true,
        ..HostingSettings::default()
    });
    let mut browser = Browser::new(open);
    let signin = browser.get(&authorize_path()).await;
    assert!(
        signin.body.contains("/signup?return_to="),
        "{}",
        signin.body
    );

    let form = browser
        .get(&format!(
            "/signup?return_to={}",
            urlencoding::encode(&authorize_path())
        ))
        .await;
    assert_eq!(form.status, StatusCode::OK);
    let created = browser
        .submit(
            "/signup",
            &form,
            &[
                ("name", "Grace"),
                ("email", "grace@example.test"),
                ("password", PASSWORD),
            ],
        )
        .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    assert!(
        created.location().starts_with("/oauth/authorize?"),
        "{}",
        created.location()
    );
    let consent = browser.get(created.location()).await;
    assert!(
        consent.body.contains("grace@example.test"),
        "signed in as the new account: {}",
        consent.body
    );
    let admin: bool =
        sqlx::query_scalar("SELECT is_admin FROM users WHERE email = 'grace@example.test'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!admin);
}

#[sqlx::test(migrations = "../migrations")]
async fn pairing_approves_new_and_existing_agents_and_denies(pool: PgPool) {
    seed(&pool).await;
    let app = state(&pool);
    let start_pairing = || async {
        let response = crate::router(state(&pool))
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/oauth/device_authorization")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"agent_display_name_hint":"Release Notes Bot"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["verification_uri_complete"]
            .as_str()
            .unwrap()
            .contains("/app/pair?session="));
        body["user_code"].as_str().unwrap().to_owned()
    };

    // Signed out: sign in first, then back to the request.
    let code = start_pairing().await;
    let mut browser = Browser::new(app);
    let signin = browser.get(&format!("/app/pair?session={code}")).await;
    assert!(signin.body.contains("approve a device pairing"));
    let done = browser
        .submit(
            "/app/pair",
            &signin,
            &[("email", "ada@example.test"), ("password", PASSWORD)],
        )
        .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    let form = browser.get(done.location()).await;
    assert!(
        form.body.contains("release-notes-bot"),
        "the slug comes from the hint"
    );

    let approved = browser
        .submit(
            "/app/pair",
            &form,
            &[
                ("step", "approve"),
                ("target", "new"),
                ("display_name", "Release Notes Bot"),
                ("slug", "release-notes-bot"),
                ("visibility", "private"),
            ],
        )
        .await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    assert!(approved.body.contains("is connected"));
    let agent: Uuid =
        sqlx::query_scalar("SELECT approved_agent_id FROM oauth_device_codes WHERE user_code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .unwrap();

    // A second device, attached to the same agent.
    let code = start_pairing().await;
    let form = browser
        .get(&format!(
            "/app/pair?session={}",
            code.to_lowercase().replace('-', "")
        ))
        .await;
    assert_eq!(form.status, StatusCode::OK, "codes are normalized");
    let approved = browser
        .submit(
            "/app/pair",
            &form,
            &[("step", "approve"), ("target", &agent.to_string())],
        )
        .await;
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.body);
    let attached: Uuid =
        sqlx::query_scalar("SELECT approved_agent_id FROM oauth_device_codes WHERE user_code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(attached, agent);

    // A third, denied.
    let code = start_pairing().await;
    let form = browser.get(&format!("/app/pair?session={code}")).await;
    let denied = browser
        .submit("/app/pair", &form, &[("step", "deny")])
        .await;
    assert_eq!(denied.status, StatusCode::OK);
    let denied_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT denied_at FROM oauth_device_codes WHERE user_code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(denied_at.is_some());

    // An unknown code.
    let page = browser.get("/app/pair?session=ZZZZ-ZZZZ").await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../migrations")]
async fn qr_codes_only_for_this_server(pool: PgPool) {
    let mut browser = Browser::new(state(&pool));
    let page = browser
        .get(&format!(
            "/qr?data={}",
            urlencoding::encode("http://localhost:8080/app/pair?session=ABCD-EFGH")
        ))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("data:image/svg+xml;base64,"));
    for elsewhere in ["https://evil.example/app/pair", "javascript:alert(1)", ""] {
        let page = browser
            .get(&format!("/qr?data={}", urlencoding::encode(elsewhere)))
            .await;
        assert_eq!(page.status, StatusCode::BAD_REQUEST, "{elsewhere}");
    }
}

#[sqlx::test(migrations = "../migrations")]
async fn signing_out_revokes_the_session(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    let consent = sign_in(&mut browser).await;
    let session = browser.cookies["chakramcp_page_session"].clone();
    let out = browser
        .post(
            "/logout",
            &[
                ("csrf".into(), consent.field("csrf")),
                ("return_to".into(), authorize_path()),
            ],
        )
        .await;
    assert_eq!(out.status, StatusCode::SEE_OTHER);
    assert!(!browser.cookies.contains_key("chakramcp_page_session"));
    let revoked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM revoked_tokens WHERE reason = 'page_signout'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(revoked, 1);

    // The old cookie no longer signs anyone in.
    browser
        .cookies
        .insert("chakramcp_page_session".into(), session);
    let page = browser.get(&authorize_path()).await;
    assert!(page.body.contains("Sign in"));
}

#[sqlx::test(migrations = "../migrations")]
async fn wrong_passwords_hit_the_limit(pool: PgPool) {
    seed(&pool).await;
    let mut browser = Browser::new(state(&pool));
    let signin = browser.get(&authorize_path()).await;
    for _ in 0..crate::signin_limit::MAX_FAILURES {
        let page = browser
            .submit(
                "/oauth/authorize",
                &signin,
                &[
                    ("email", "ada@example.test"),
                    ("password", "nope-nope-nope"),
                ],
            )
            .await;
        assert_eq!(page.status, StatusCode::UNAUTHORIZED);
    }
    let page = browser
        .submit(
            "/oauth/authorize",
            &signin,
            &[("email", "ada@example.test"), ("password", PASSWORD)],
        )
        .await;
    assert_eq!(
        page.status,
        StatusCode::TOO_MANY_REQUESTS,
        "even the right password"
    );
    assert!(page.body.contains("Too many failed attempts"));
}

#[sqlx::test(migrations = "../migrations")]
async fn managed_servers_serve_no_pages(pool: PgPool) {
    let managed = state(&pool).with_hosting(HostingSettings::for_mode(HostingMode::Managed));
    let mut browser = Browser::new(managed);
    for path in [
        "/oauth/authorize",
        "/signup",
        "/app/pair",
        "/qr",
        "/assets/pages.css",
    ] {
        assert_eq!(
            browser.get(path).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[test]
fn return_to_stays_on_this_server() {
    let state_less = super::security::AppOrigin::of_url_for_tests(ORIGIN);
    assert_eq!(
        state_less
            .return_path("/oauth/authorize?client_id=x")
            .as_deref(),
        Some("/oauth/authorize?client_id=x")
    );
    assert_eq!(
        state_less.return_path("/app/pair").as_deref(),
        Some("/app/pair")
    );
    for trick in [
        "//evil.example/x",
        "/\\evil.example",
        "/%09/evil.example",
        "/%5Cevil.example",
        "https://evil.example/",
        "javascript:alert(1)",
        "relative/path",
        "",
    ] {
        assert_eq!(state_less.return_path(trick), None, "{trick:?}");
    }
}
