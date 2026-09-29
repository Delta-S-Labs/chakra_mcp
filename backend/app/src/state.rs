use std::sync::Arc;

use sqlx::PgPool;

use chakramcp_shared::config::SharedConfig;
use chakramcp_shared::credits::CreditsConfig;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<SharedConfig>,
    /// Shared with the frontend's server-side sign-in callback, the only
    /// caller allowed to exchange a provider identity for a session
    /// (`UPSERT_SHARED_SECRET`). `None` refuses every such sign-in.
    pub upsert_secret: Option<Arc<str>>,
    /// The global credit defaults, for showing effective grants and limits.
    /// Must match what the relay enforces: both mains read the same env.
    pub credits: CreditsConfig,
}

impl AppState {
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
            upsert_secret: None,
            credits: CreditsConfig::default(),
        }
    }

    /// Blank counts as unset.
    pub fn with_upsert_secret(mut self, secret: Option<String>) -> Self {
        self.upsert_secret = secret
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .map(Arc::from);
        self
    }

    pub fn with_credits_config(mut self, credits: CreditsConfig) -> Self {
        self.credits = credits;
        self
    }

    pub fn admin_email(&self) -> Option<&str> {
        self.config.admin_email.as_deref()
    }
}
