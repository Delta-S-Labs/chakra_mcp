use std::sync::Arc;

use sqlx::PgPool;

use chakramcp_shared::config::SharedConfig;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<SharedConfig>,
    /// Shared with the frontend's server-side sign-in callback, the only
    /// caller allowed to exchange a provider identity for a session
    /// (`UPSERT_SHARED_SECRET`). `None` refuses every such sign-in.
    pub upsert_secret: Option<Arc<str>>,
}

impl AppState {
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
            upsert_secret: None,
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

    pub fn admin_email(&self) -> Option<&str> {
        self.config.admin_email.as_deref()
    }
}
