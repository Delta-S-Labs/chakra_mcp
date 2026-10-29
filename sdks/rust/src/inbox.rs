//! Inbox client + the long-running `serve` loop.

use std::future::Future;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::client::{pull_path, ChakraMCP};
use crate::error::Result;
use crate::types::{DraftedReply, InboxPullQuery, Invocation, Notice, PullQuery, PullResult};

#[derive(Debug, Clone)]
pub enum HandlerResult {
    Succeeded(Value),
    Failed(String),
}

/// What `respond_*` returns.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RespondOutcome {
    /// The call finished with this reply.
    Finished(Box<Invocation>),
    /// A reply to a `human_in_loop` capability, kept as a draft until one
    /// of the agent's owners confirms it at `approve_url`.
    Drafted(DraftedReply),
}

impl RespondOutcome {
    fn from_value(v: Value) -> Result<Self> {
        let drafted = v.get("status").and_then(Value::as_str) == Some("awaiting_confirmation");
        Ok(if drafted {
            Self::Drafted(serde_json::from_value(v)?)
        } else {
            Self::Finished(Box::new(serde_json::from_value(v)?))
        })
    }
}

pub struct InboxClient<'a> {
    parent: &'a ChakraMCP,
}

impl<'a> InboxClient<'a> {
    pub(crate) fn new(parent: &'a ChakraMCP) -> Self {
        Self { parent }
    }

    /// Atomically claim the oldest pending invocations targeting an
    /// agent you own. Concurrent pullers (across machines) get
    /// disjoint batches via `FOR UPDATE SKIP LOCKED` at the DB.
    ///
    /// Takes an [`InboxPullQuery`], or a bare `Option<u32>` limit. With
    /// `wait`, the relay holds the pull until work arrives or the wait
    /// ends; relays before 0.7 answer at once.
    pub async fn pull(
        &self,
        agent_id: &str,
        query: impl Into<InboxPullQuery>,
    ) -> Result<Vec<Invocation>> {
        Ok(self.pull_with_meta(agent_id, query).await?.items)
    }

    /// [`Self::pull`], plus how long the relay waited, for agents that run
    /// their own loop. After an empty answer with `waited: Some(n)`, n > 0,
    /// pull again at once; otherwise the relay didn't wait, so pause first.
    pub async fn pull_with_meta(
        &self,
        agent_id: &str,
        query: impl Into<InboxPullQuery>,
    ) -> Result<PullResult<Invocation>> {
        let q = query.into();
        let path = pull_path(
            "/v1/inbox",
            agent_id,
            &[
                ("limit", q.limit),
                ("wait", q.wait),
                ("lease_secs", q.lease_secs),
            ],
        );
        self.parent.relay_pull(&path, q.wait).await
    }

    /// Report success for an in_progress invocation you claimed. For a
    /// `human_in_loop` capability the reply comes back
    /// [`RespondOutcome::Drafted`].
    pub async fn respond_succeeded(
        &self,
        invocation_id: &str,
        output: Value,
    ) -> Result<RespondOutcome> {
        self.respond(
            invocation_id,
            &json!({ "status": "succeeded", "output": output }),
        )
        .await
    }

    /// Report failure; see [`Self::respond_succeeded`].
    pub async fn respond_failed(&self, invocation_id: &str, error: &str) -> Result<RespondOutcome> {
        self.respond(
            invocation_id,
            &json!({ "status": "failed", "error": error }),
        )
        .await
    }

    /// Hand back an invocation you claimed and won't answer (relays 0.7+):
    /// it goes back to pending for the next pull, and doesn't count as a
    /// delivery. Refused with 409 `chk.invocation.not_releasable` while it
    /// isn't claimed or has a reply waiting for its owner, and
    /// `chk.invocation.already_settled` once it's settled.
    pub async fn release(&self, invocation_id: &str) -> Result<Invocation> {
        self.parent
            .relay_post(
                &format!("/v1/invocations/{}/release", urlencode(invocation_id)),
                &json!({}),
            )
            .await
    }

    async fn respond(&self, invocation_id: &str, body: &Value) -> Result<RespondOutcome> {
        let v: Value = self
            .parent
            .relay_post(
                &format!("/v1/invocations/{}/result", urlencode(invocation_id)),
                body,
            )
            .await?;
        RespondOutcome::from_value(v)
    }

    /// Claim the agent's notices, oldest first; each is delivered once. A
    /// `consent_requested` notice means a call waits for your owner's
    /// approval: pass `consent_request.approve_url` on to them.
    ///
    /// Takes a [`PullQuery`], or a bare `Option<u32>` limit. With `wait`,
    /// the relay holds the pull until a notice arrives or the wait ends;
    /// relays before 0.7 answer at once.
    pub async fn notices(
        &self,
        agent_id: &str,
        query: impl Into<PullQuery>,
    ) -> Result<Vec<Notice>> {
        let q = query.into();
        let path = pull_path(
            "/v1/inbox/notices",
            agent_id,
            &[("limit", q.limit), ("wait", q.wait)],
        );
        Ok(self.parent.relay_pull(&path, q.wait).await?.items)
    }

    /// Long-running pull → handler → respond loop. Returns a builder so
    /// you can configure and either `.await` it or attach a
    /// CancellationToken first.
    ///
    /// The handler is `FnMut(Invocation) -> Future<Output = Result<HandlerResult, _>>`.
    /// Errors raised by the handler - both Err returns and panics
    /// turned into errors - are reported as `failed` invocations and
    /// the loop continues.
    ///
    /// Each pull asks the relay to wait for work ([`ServeBuilder::wait_secs`],
    /// 25 s by default). While it waits, the loop pulls again straight
    /// after an empty answer, so work is handled as it arrives. A relay
    /// that didn't wait, such as one before 0.7, gets
    /// [`ServeBuilder::poll_interval`] between pulls.
    pub fn serve<F, Fut, E>(
        self,
        agent_id: impl Into<String>,
        handler: F,
    ) -> ServeBuilder<'a, F, Fut, E>
    where
        F: FnMut(Invocation) -> Fut + Send + 'a,
        Fut: Future<Output = std::result::Result<HandlerResult, E>> + Send + 'a,
        E: std::fmt::Display + Send + 'a,
    {
        ServeBuilder {
            inbox: self,
            agent_id: agent_id.into(),
            handler,
            poll_interval: Duration::from_secs(2),
            batch_size: 25,
            wait_secs: DEFAULT_WAIT_SECS,
            lease_secs: None,
            cancel: None,
            _phantom: std::marker::PhantomData,
        }
    }
}

pub struct ServeBuilder<'a, F, Fut, E> {
    inbox: InboxClient<'a>,
    agent_id: String,
    handler: F,
    poll_interval: Duration,
    batch_size: u32,
    wait_secs: u32,
    lease_secs: Option<u32>,
    cancel: Option<CancellationToken>,
    _phantom: std::marker::PhantomData<fn() -> (Fut, E)>,
}

impl<'a, F, Fut, E> ServeBuilder<'a, F, Fut, E>
where
    F: FnMut(Invocation) -> Fut + Send + 'a,
    Fut: Future<Output = std::result::Result<HandlerResult, E>> + Send + 'a,
    E: std::fmt::Display + Send + 'a,
{
    /// How long to wait after an empty pull the relay didn't wait on, and
    /// after a failed pull. Default 2 s.
    pub fn poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }
    pub fn batch_size(mut self, n: u32) -> Self {
        self.batch_size = n;
        self
    }
    /// Seconds each pull asks the relay to wait for work; 25 by default,
    /// the most a relay honours unless its operator allows more. 0 never
    /// waits: each empty pull is followed by `poll_interval`, as before 0.7.
    pub fn wait_secs(mut self, secs: u32) -> Self {
        self.wait_secs = secs;
        self
    }
    /// How long each claim lasts before the relay may hand the invocation
    /// out again, for handlers that run longer than its default lease.
    /// Relays that don't support leases ignore it.
    pub fn lease_secs(mut self, secs: u32) -> Self {
        self.lease_secs = Some(secs);
        self
    }
    /// Stop the loop when `token` fires, also mid-pull.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Run the loop. Returns when the cancellation token fires.
    pub async fn run(mut self) -> Result<()> {
        let cancel = self.cancel.unwrap_or_default();
        let query = InboxPullQuery {
            limit: Some(self.batch_size),
            wait: (self.wait_secs > 0).then_some(self.wait_secs),
            lease_secs: self.lease_secs,
        };
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let pulled = tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                r = self.inbox.pull_with_meta(&self.agent_id, query) => r,
            };
            let batch = match pulled {
                Ok(PullResult { items, waited }) => {
                    if items.is_empty() && waited.is_some_and(|n| n > 0) {
                        // The relay waited and nothing came: pull again.
                        continue;
                    }
                    items
                }
                // Transient pull errors - sleep and retry.
                Err(_) => Vec::new(),
            };
            if batch.is_empty() {
                // A failed pull, or an empty one the relay didn't wait on
                // (an older relay, or one with no waiting slot free).
                if cancel
                    .run_until_cancelled(tokio::time::sleep(self.poll_interval))
                    .await
                    .is_none()
                {
                    return Ok(());
                }
                continue;
            }
            for inv in batch {
                if cancel.is_cancelled() {
                    return Ok(());
                }
                let id = inv.id.clone();
                let result = (self.handler)(inv).await;
                match result {
                    Ok(HandlerResult::Succeeded(out)) => {
                        let _ = self.inbox.respond_succeeded(&id, out).await;
                    }
                    Ok(HandlerResult::Failed(msg)) => {
                        let _ = self.inbox.respond_failed(&id, &msg).await;
                    }
                    Err(e) => {
                        let _ = self.inbox.respond_failed(&id, &e.to_string()).await;
                    }
                }
            }
        }
    }
}

impl<'a, F, Fut, E> std::future::IntoFuture for ServeBuilder<'a, F, Fut, E>
where
    F: FnMut(Invocation) -> Fut + Send + 'a,
    Fut: Future<Output = std::result::Result<HandlerResult, E>> + Send + 'a,
    E: std::fmt::Display + Send + 'a,
{
    type Output = Result<()>;
    type IntoFuture = std::pin::Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

/// What `serve` loops ask the relay to wait for work, in seconds.
pub(crate) const DEFAULT_WAIT_SECS: u32 = 25;

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
