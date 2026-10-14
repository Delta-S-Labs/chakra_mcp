//! The pages' templates (`app/templates/pages/`). Askama escapes every
//! value as HTML.

use askama::Template;

pub(crate) struct Hidden {
    pub name: &'static str,
    pub value: String,
}

pub(crate) struct AgentOption {
    pub id: String,
    pub slug: String,
    pub display_name: String,
    pub account_slug: String,
    pub checked: bool,
}

pub(crate) struct Link {
    pub href: String,
    pub text: String,
}

#[derive(Template)]
#[template(path = "pages/signin.html")]
pub(crate) struct SigninPage {
    pub title: String,
    pub host: String,
    pub lead: String,
    pub action: &'static str,
    pub csrf: String,
    pub hidden: Vec<Hidden>,
    pub email: String,
    pub error: Option<String>,
    pub signup_href: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/consent.html")]
pub(crate) struct ConsentPage {
    pub title: String,
    pub host: String,
    pub client_name: String,
    pub client_uri: Option<String>,
    pub redirect_host: String,
    pub user_email: String,
    pub csrf: String,
    pub hidden: Vec<Hidden>,
    pub agent_scope: String,
    pub agents: Vec<AgentOption>,
    pub error: Option<String>,
    pub return_to: String,
}

#[derive(Template)]
#[template(path = "pages/signup.html")]
pub(crate) struct SignupPage {
    pub title: String,
    pub host: String,
    pub csrf: String,
    pub return_to: String,
    pub name: String,
    pub email: String,
    pub error: Option<String>,
    pub signin_href: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/pair.html")]
pub(crate) struct PairPage {
    pub title: String,
    pub host: String,
    pub user_email: String,
    pub user_code: String,
    pub persona: Option<String>,
    pub csrf: String,
    pub agents: Vec<AgentOption>,
    pub new_agent: bool,
    pub display_name: String,
    pub slug: String,
    pub description: String,
    pub visibility: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/pair_code.html")]
pub(crate) struct PairCodePage {
    pub title: String,
    pub host: String,
    pub code: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/qr.html")]
pub(crate) struct QrPage {
    pub title: String,
    pub host: String,
    /// A `data:image/svg+xml;base64,…` URL.
    pub image: String,
    pub url: String,
}

#[derive(Template)]
#[template(path = "pages/message.html")]
pub(crate) struct MessagePage {
    pub title: String,
    pub host: String,
    pub heading: String,
    pub message: String,
    pub link: Option<Link>,
}
