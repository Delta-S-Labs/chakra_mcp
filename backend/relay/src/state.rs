use std::sync::Arc;

use sqlx::PgPool;

use chakramcp_shared::config::SharedConfig;

#[derive(Clone)]
pub struct RelayState {
    pub db: PgPool,
    #[allow(dead_code)]
    pub config: Arc<SharedConfig>,
}

impl RelayState {
    pub fn new(db: PgPool, config: SharedConfig) -> Self {
        Self {
            db,
            config: Arc::new(config),
        }
    }

    pub fn jwt_secret(&self) -> &str {
        &self.config.jwt_secret
    }
}
