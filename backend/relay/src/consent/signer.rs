//! The signing key for consent tokens, cached in memory.
//!
//! Tokens are minted at delivery, not on the invocation path, but an inbox
//! claim still shouldn't read the key from the database each time. The key
//! is re-read every few minutes, which is enough to notice a rotation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::agent_card::keys::{KeyStore, KeyStoreError};
use crate::agent_card::signer::SigningKey;

/// How long a cached key is used before it's read again.
pub const KEY_REFRESH: Duration = Duration::from_secs(5 * 60);

pub struct ConsentSigner {
    keystore: KeyStore,
    cached: RwLock<Option<(Arc<SigningKey>, Instant)>>,
}

impl ConsentSigner {
    pub fn new(db: PgPool) -> Self {
        Self {
            keystore: KeyStore::new(db),
            cached: RwLock::new(None),
        }
    }

    /// The active signing key, creating one if the relay has none yet.
    pub async fn key(&self) -> Result<Arc<SigningKey>, KeyStoreError> {
        if let Some((key, read_at)) = &*self.cached.read().await {
            if read_at.elapsed() < KEY_REFRESH {
                return Ok(key.clone());
            }
        }
        let key = Arc::new(self.keystore.ensure_active_key().await?);
        *self.cached.write().await = Some((key.clone(), Instant::now()));
        Ok(key)
    }
}
