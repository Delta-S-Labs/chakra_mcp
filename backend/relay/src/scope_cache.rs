//! Which agents a credential may act as, cached in memory (MCP and discovery
//! spec §4.2).
//!
//! A credential's agent scope (`all`, `own` or `selected`, migration 0027)
//! narrows every action taken as an agent, invoke included. Resolving it costs
//! queries, and the invoke path reads only in-memory state, so the answer is
//! cached per credential for 30 seconds:
//!
//! - **Key:** the token's `jti`, or the API key's id: what `resolve_grant`
//!   looks up. Each token has its own scope row, so no two share an entry.
//! - **Value:** `None` for all agents, else the agent ids in scope, limited to
//!   accounts the user belongs to (`own` matches a client's agents across all
//!   of its users).
//! - **A refusal is re-checked** against the database before it's returned, so
//!   an agent added elsewhere (another replica, the app) is never refused from
//!   a stale entry. Refusals are rare; allowed calls never query once cached.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chakramcp_shared::error::ApiError;
use sqlx::PgPool;
use uuid::Uuid;

const TTL: Duration = Duration::from_secs(30);
/// Past this many entries, expired ones are dropped on the next insert.
const MAX_ENTRIES: usize = 10_000;

/// The agents a credential may act as: `None` for all of them.
pub type InScope = Option<Arc<HashSet<Uuid>>>;

/// A credential, as `resolve_grant` identifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Credential {
    pub user_id: Uuid,
    pub minted_jti: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Jwt(Uuid),
    ApiKey(Uuid),
}

impl Credential {
    fn key(&self) -> Option<Key> {
        self.minted_jti
            .map(Key::Jwt)
            .or(self.api_key_id.map(Key::ApiKey))
    }
}

pub struct ScopeCache {
    ttl: Duration,
    entries: Mutex<HashMap<Key, (InScope, Instant)>>,
}

impl Default for ScopeCache {
    fn default() -> Self {
        Self::with_ttl(TTL)
    }
}

impl ScopeCache {
    /// A cache whose entries live `ttl` (tests use zero).
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::default(),
        }
    }

    /// The agents `cred` may act as.
    pub async fn in_scope(&self, db: &PgPool, cred: Credential) -> Result<InScope, ApiError> {
        if let Some(hit) = cred.key().and_then(|k| self.get(k)) {
            return Ok(hit);
        }
        self.reload(db, cred).await
    }

    /// Whether `cred` may act as `agent_id`.
    pub async fn allows(
        &self,
        db: &PgPool,
        cred: Credential,
        agent_id: Uuid,
    ) -> Result<bool, ApiError> {
        if allows(&self.in_scope(db, cred).await?, agent_id) {
            return Ok(true);
        }
        // Refused from the cache: look again before saying no.
        Ok(allows(&self.reload(db, cred).await?, agent_id))
    }

    /// Drop `cred`'s entry: it just gained an agent.
    pub fn forget(&self, cred: Credential) {
        if let Some(k) = cred.key() {
            self.entries
                .lock()
                .expect("scope cache poisoned")
                .remove(&k);
        }
    }

    async fn reload(&self, db: &PgPool, cred: Credential) -> Result<InScope, ApiError> {
        let fresh = load(db, cred).await?;
        if let Some(k) = cred.key() {
            let mut entries = self.entries.lock().expect("scope cache poisoned");
            if entries.len() >= MAX_ENTRIES {
                entries.retain(|_, (_, at)| at.elapsed() < self.ttl);
            }
            entries.insert(k, (fresh.clone(), Instant::now()));
        }
        Ok(fresh)
    }

    fn get(&self, k: Key) -> Option<InScope> {
        let entries = self.entries.lock().expect("scope cache poisoned");
        entries
            .get(&k)
            .filter(|(_, at)| at.elapsed() < self.ttl)
            .map(|(v, _)| v.clone())
    }
}

/// Whether `scope` covers `agent_id`.
pub fn allows(scope: &InScope, agent_id: Uuid) -> bool {
    match scope {
        None => true,
        Some(ids) => ids.contains(&agent_id),
    }
}

/// Whether a relationship between agents `a` and `b` is in scope: either side.
pub fn allows_either(scope: &InScope, a: Uuid, b: Uuid) -> bool {
    allows(scope, a) || allows(scope, b)
}

async fn load(db: &PgPool, cred: Credential) -> Result<InScope, ApiError> {
    let grant =
        chakramcp_shared::scope::resolve_grant(db, cred.minted_jti, cred.api_key_id).await?;
    let Some(ids) = chakramcp_shared::scope::manageable_agent_ids(db, &grant).await? else {
        return Ok(None);
    };
    let mine = sqlx::query_scalar!(
        r#"SELECT a.id FROM agents a
             JOIN account_memberships m ON m.account_id = a.account_id AND m.user_id = $2
            WHERE a.id = ANY($1)"#,
        &ids,
        cred.user_id,
    )
    .fetch_all(db)
    .await?;
    Ok(Some(Arc::new(mine.into_iter().collect())))
}
