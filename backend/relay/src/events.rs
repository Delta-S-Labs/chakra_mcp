//! General usage + audit event recording.
//!
//! `usage_events` meters EVERY request (REST routes + each MCP tool),
//! including read-only GETs/pulls — the substrate for future billing.
//! `audit_events` records every WRITE (create/update/delete/state change);
//! read-only pulls are intentionally not audited.
//!
//! Both recorders are best-effort: a failure to write an event must never
//! fail the user's request, so errors are logged and swallowed.
//!
//! Usage metering also must never *slow* a request: request paths hand
//! events to a [`UsageRecorder`], which only does a non-blocking channel
//! send. A background writer resolves attribution and does the INSERTs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::state::RelayState;

/// Record one metered request. `actor` is None for unauthenticated hits
/// (e.g. public discovery). `account_id` is optional — usage is primarily
/// attributed by user/key, and the owning account isn't always known at
/// the middleware layer.
#[allow(clippy::too_many_arguments)]
pub async fn record_usage(
    db: &PgPool,
    actor: Option<&AuthUser>,
    account_id: Option<Uuid>,
    surface: &str,
    action: &str,
    method: &str,
    route: &str,
    status_code: i32,
) {
    let (uid, api_key_id, jti) = match actor {
        Some(a) => (Some(a.user_id), a.api_key_id, a.minted_jti),
        None => (None, None, None),
    };
    let ok = (200..400).contains(&status_code);
    let res = sqlx::query!(
        r#"
        INSERT INTO usage_events
            (id, actor_user_id, account_id, api_key_id, minted_jti,
             surface, action, method, route, status_code, ok)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        Uuid::now_v7(),
        uid,
        account_id,
        api_key_id,
        jti,
        surface,
        action,
        method,
        route,
        status_code,
        ok,
    )
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("usage_events insert failed: {e}");
    }
}

/// Record one write to the audit trail. Only called from write paths
/// (create/update/delete/state change) — never from reads.
#[allow(clippy::too_many_arguments)]
pub async fn record_audit(
    db: &PgPool,
    actor: &AuthUser,
    account_id: Option<Uuid>,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    target_id: Option<Uuid>,
    summary: &str,
    metadata: Value,
) {
    let res = sqlx::query!(
        r#"
        INSERT INTO audit_events
            (id, actor_user_id, account_id, api_key_id, minted_jti,
             action, resource_type, resource_id, target_id, summary, metadata)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        Uuid::now_v7(),
        actor.user_id,
        account_id,
        actor.api_key_id,
        actor.minted_jti,
        action,
        resource_type,
        resource_id,
        target_id,
        summary,
        metadata,
    )
    .execute(db)
    .await;
    if let Err(e) = res {
        tracing::warn!("audit_events insert failed: {e}");
    }
}

// ─── Off-request-path usage recording ────────────────────────

/// Queue capacity. If the writer falls this far behind, new events are
/// dropped (and counted) rather than making requests wait.
pub const USAGE_CHANNEL_CAPACITY: usize = 10_000;

/// Upper bound on events handled per writer wake-up.
const USAGE_BATCH_MAX: usize = 500;

/// Who made a metered request, as far as the request path knows.
#[derive(Debug, Clone)]
pub enum UsageActor {
    /// No credentials were presented.
    Anonymous,
    /// The raw `Authorization` header, resolved by the writer so the request
    /// never waits on an auth lookup just to attribute usage.
    Header(String),
    /// Already authenticated by the handler (e.g. MCP tool calls).
    User(AuthUser),
}

/// One metered request.
#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub actor: UsageActor,
    pub account_id: Option<Uuid>,
    pub surface: &'static str,
    pub action: String,
    pub method: String,
    pub route: String,
    pub status_code: i32,
}

enum UsageMsg {
    Event(UsageEvent),
    Flush(oneshot::Sender<()>),
}

/// Records usage without ever blocking the caller. The default records
/// nothing; production attaches one from [`UsageRecorder::spawn`].
#[derive(Clone, Default)]
pub struct UsageRecorder {
    inner: Option<Arc<RecorderInner>>,
}

struct RecorderInner {
    tx: mpsc::Sender<UsageMsg>,
    dropped: AtomicU64,
}

impl UsageRecorder {
    /// A recorder that discards every event.
    pub fn noop() -> Self {
        Self::default()
    }

    /// Spawn the background writer and return a recorder that feeds it.
    /// `state` gives the writer its pool and the JWT secret for attribution.
    pub fn spawn(state: RelayState) -> Self {
        let (tx, rx) = mpsc::channel(USAGE_CHANNEL_CAPACITY);
        tokio::spawn(run_usage_writer(state, rx));
        Self::from_sender(tx)
    }

    fn from_sender(tx: mpsc::Sender<UsageMsg>) -> Self {
        Self {
            inner: Some(Arc::new(RecorderInner {
                tx,
                dropped: AtomicU64::new(0),
            })),
        }
    }

    /// Queue one event without waiting. If the writer is behind and the
    /// queue is full, the event is dropped and counted: usage is best-effort,
    /// and a request must never wait on it.
    pub fn record(&self, event: UsageEvent) {
        let Some(inner) = &self.inner else {
            return;
        };
        if inner.tx.try_send(UsageMsg::Event(event)).is_err() {
            let dropped = inner.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if dropped == 1 || dropped % 1_000 == 0 {
                tracing::warn!(dropped, "usage writer is behind; dropping usage events");
            }
        }
    }

    /// Events dropped because the queue was full or the writer was gone.
    pub fn dropped(&self) -> u64 {
        self.inner
            .as_ref()
            .map_or(0, |inner| inner.dropped.load(Ordering::Relaxed))
    }

    /// Wait until every event queued before this call has been written.
    /// Tests use it to assert on `usage_events`; request paths never call it.
    pub async fn flush(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        let (done_tx, done_rx) = oneshot::channel();
        if inner.tx.send(UsageMsg::Flush(done_tx)).await.is_ok() {
            let _ = done_rx.await;
        }
    }
}

/// Drains the queue, handling whatever has arrived (up to
/// [`USAGE_BATCH_MAX`]) per wake-up. Uses one pooled connection at a time,
/// so it never competes with request handlers for the pool.
async fn run_usage_writer(state: RelayState, mut rx: mpsc::Receiver<UsageMsg>) {
    let mut batch = Vec::with_capacity(USAGE_BATCH_MAX);
    let mut flushes = Vec::new();
    while let Some(msg) = rx.recv().await {
        push_usage_msg(msg, &mut batch, &mut flushes);
        while batch.len() < USAGE_BATCH_MAX {
            match rx.try_recv() {
                Ok(msg) => push_usage_msg(msg, &mut batch, &mut flushes),
                Err(_) => break,
            }
        }
        write_usage_batch(&state, &batch).await;
        batch.clear();
        for done in flushes.drain(..) {
            let _ = done.send(());
        }
    }
}

fn push_usage_msg(
    msg: UsageMsg,
    batch: &mut Vec<UsageEvent>,
    flushes: &mut Vec<oneshot::Sender<()>>,
) {
    match msg {
        UsageMsg::Event(event) => batch.push(event),
        UsageMsg::Flush(done) => flushes.push(done),
    }
}

/// Resolve each distinct `Authorization` header once, then insert every
/// row. Rows are inserted independently rather than in one transaction: a
/// row whose user or API key was deleted in the meantime fails its own FK
/// check without taking the rest of the batch with it.
async fn write_usage_batch(state: &RelayState, batch: &[UsageEvent]) {
    let mut resolved: HashMap<&str, Option<AuthUser>> = HashMap::new();
    for event in batch {
        if let UsageActor::Header(header) = &event.actor {
            if !resolved.contains_key(header.as_str()) {
                let user = crate::auth::authenticate(state, Some(header)).await;
                resolved.insert(header.as_str(), user);
            }
        }
    }
    for event in batch {
        let actor = match &event.actor {
            UsageActor::Anonymous => None,
            UsageActor::Header(header) => resolved.get(header.as_str()).and_then(Option::as_ref),
            UsageActor::User(user) => Some(user),
        };
        record_usage(
            &state.db,
            actor,
            event.account_id,
            event.surface,
            &event.action,
            &event.method,
            &event.route,
            event.status_code,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anonymous_event(route: &str) -> UsageEvent {
        UsageEvent {
            actor: UsageActor::Anonymous,
            account_id: None,
            surface: "rest",
            action: format!("GET {route}"),
            method: "GET".to_owned(),
            route: route.to_owned(),
            status_code: 200,
        }
    }

    #[tokio::test]
    async fn noop_recorder_discards_without_error() {
        let recorder = UsageRecorder::noop();
        for _ in 0..10 {
            recorder.record(anonymous_event("/x"));
        }
        recorder.flush().await;
        assert_eq!(recorder.dropped(), 0);
    }

    #[tokio::test]
    async fn full_queue_drops_instead_of_blocking() {
        // Hold the receiver without reading it: the writer is "stuck".
        let (tx, _rx) = mpsc::channel(1);
        let recorder = UsageRecorder::from_sender(tx);
        recorder.record(anonymous_event("/a")); // fills the queue
        recorder.record(anonymous_event("/b")); // dropped
        recorder.record(anonymous_event("/c")); // dropped
        assert_eq!(recorder.dropped(), 2);
    }
}
