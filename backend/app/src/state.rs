use std::sync::Arc;

use sqlx::PgPool;

use chakramcp_shared::config::SharedConfig;
use chakramcp_shared::credits::CreditsConfig;
use chakramcp_shared::hosting::HostingSettings;

use crate::purchases::PurchaseConfig;

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
    /// Who may sign up and who is an admin (`HOSTING_MODE` and its
    /// overrides). `new` starts from the self-hosted defaults; the mains set
    /// it from config.
    pub hosting: HostingSettings,
    /// Buying credits through Dodo (credits P4): set only on chakramcp.com,
    /// with credits on and Dodo configured. `None` turns the checkout and
    /// webhook routes into 404s.
    pub purchase: Option<Arc<PurchaseConfig>>,
}

impl AppState {
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
            upsert_secret: None,
            credits: CreditsConfig::default(),
            hosting: HostingSettings::default(),
            purchase: None,
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

    pub fn with_hosting(mut self, hosting: HostingSettings) -> Self {
        self.hosting = hosting;
        self
    }

    pub fn with_purchase(mut self, purchase: Option<PurchaseConfig>) -> Self {
        self.purchase = purchase.map(Arc::new);
        self
    }

    pub fn admin_email(&self) -> Option<&str> {
        self.config.admin_email.as_deref()
    }
}
