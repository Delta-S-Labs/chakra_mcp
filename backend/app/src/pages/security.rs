//! Response headers, the server's own origin, and the checks that keep
//! redirects and QR codes on it.

use askama::Template;
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use url::Url;

use super::views::{Link, MessagePage};
use crate::state::AppState;

/// This server's public origin, from its app URL: forms must come from it,
/// `return_to` must stay on it, and `/qr` only encodes links to it.
#[derive(Debug, Clone)]
pub(crate) struct AppOrigin {
    url: Url,
    origin: String,
}

impl AppOrigin {
    pub fn of(state: &AppState) -> Self {
        let url = Url::parse(&state.config.app_base_url)
            .unwrap_or_else(|_| Url::parse("http://localhost:8080").expect("a valid URL"));
        let origin = url.origin().ascii_serialization();
        Self { url, origin }
    }

    #[cfg(test)]
    pub fn of_url_for_tests(url: &str) -> Self {
        let url = Url::parse(url).expect("a valid URL");
        let origin = url.origin().ascii_serialization();
        Self { url, origin }
    }

    /// `https://app.example.com`
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// `app.example.com`, with the port when it isn't the default: shown on
    /// every page, so people can see which server they're signing in to.
    pub fn host(&self) -> String {
        let host = self.url.host_str().unwrap_or_default();
        match self.url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        }
    }

    /// Cookies get `Secure` when the server is reached over https.
    pub fn secure(&self) -> bool {
        self.url.scheme() == "https"
    }

    /// A path to come back to after a step, if it's a path on this server.
    /// Refuses anything that could leave it: other schemes and hosts,
    /// `//host`, backslashes, and control characters, raw or
    /// percent-encoded (`/%09/evil.com` becomes `//evil.com` in a browser).
    pub fn return_path(&self, raw: &str) -> Option<String> {
        let raw = raw.trim();
        let decoded = urlencoding::decode(raw).ok()?;
        let suspicious = |s: &str| {
            !s.starts_with('/')
                || s.starts_with("//")
                || s.contains('\\')
                || s.chars().any(char::is_control)
        };
        if suspicious(raw) || suspicious(&decoded) {
            return None;
        }
        let joined = self.url.join(raw).ok()?;
        if joined.origin().ascii_serialization() != self.origin {
            return None;
        }
        Some(match joined.query() {
            Some(query) => format!("{}?{query}", joined.path()),
            None => joined.path().to_owned(),
        })
    }

    /// Whether `url` is a link to this server.
    pub fn owns(&self, url: &str) -> bool {
        Url::parse(url)
            .map(|u| u.origin().ascii_serialization() == self.origin)
            .unwrap_or(false)
    }
}

/// The origin of a redirect URI, for the consent page's `form-action`.
pub(crate) fn origin_of(uri: &str) -> Option<String> {
    let url = Url::parse(uri).ok()?;
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Render a page with the pages' headers. `form_action` adds origins the
/// page's forms may end up at: the consent page's approve step redirects
/// to the client, and browsers apply `form-action` to that redirect too.
pub(crate) fn render(status: StatusCode, page: &impl Template, form_action: &[String]) -> Response {
    let html = match page.render() {
        Ok(html) => html,
        Err(e) => {
            tracing::error!(error = %e, "sign-in page failed to render");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "the page failed to render",
            )
                .into_response();
        }
    };
    let mut actions = String::from("'self'");
    for origin in form_action {
        actions.push(' ');
        actions.push_str(origin);
    }
    let csp = format!(
        "default-src 'none'; style-src 'self'; img-src 'self' data:; \
         frame-ancestors 'none'; base-uri 'none'; form-action {actions}"
    );
    let mut response = (status, Html(html)).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, value);
    }
    for (name, value) in [
        (header::X_FRAME_OPTIONS, "DENY"),
        // Not `no-referrer`: under it, browsers send `Origin: null` even on
        // same-origin form posts, and the CSRF check refuses those. With
        // `same-origin`, nothing reaches the client's redirect or website.
        (header::REFERRER_POLICY, "same-origin"),
        (header::CACHE_CONTROL, "no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// A page with a heading, a sentence and maybe a link.
pub(crate) fn message(
    origin: &AppOrigin,
    status: StatusCode,
    heading: &str,
    message: &str,
    link: Option<Link>,
) -> Response {
    render(
        status,
        &MessagePage {
            title: heading.to_owned(),
            host: origin.host(),
            heading: heading.to_owned(),
            message: message.to_owned(),
            link,
        },
        &[],
    )
}

/// `GET /assets/pages.css`
pub(crate) async fn stylesheet() -> Response {
    let mut response = include_str!("../../assets/pages.css").into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response
}
