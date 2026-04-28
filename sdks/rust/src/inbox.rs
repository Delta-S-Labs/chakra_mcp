//! Inbox client + the long-running `serve` loop.

use std::future::Future;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::client::ChakraMCP;
use crate::error::Result;
use crate::types::Invocation;

#[derive(Debug, Clone)]
pub enum HandlerResult {
    Succeeded(Value),
    Failed(String),
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
    pub async fn pull(&self, agent_id: &str, limit: Option<u32>) -> Result<Vec<Invocation>> {
        let mut path = format!("/v1/inbox?agent_id={}", urlencode(agent_id));
        if let Some(n) = limit {
            path.push_str(&format!("&limit={n}"));
        }
        self.parent.relay_get(&path).await
    }

    pub async fn respond_succeeded(
        &self,
        invocation_id: &str,
        output: Value,
    ) -> Result<Invocation> {
        self.parent
            .relay_post(
                &format!("/v1/invocations/{}/result", urlencode(invocation_id)),
                &json!({ "status": "succeeded", "output": output }),
            )
            .await
    }

    pub async fn respond_failed(
        &self,
        invocation_id: &str,
        error: &str,
    ) -> Result<Invocation> {
        self.parent
            .relay_post(
                &format!("/v1/invocations/{}/result", urlencode(invocation_id)),
                &json!({ "status": "failed", "error": error }),
            )
            .await
    }

    /// Long-running pull → handler → respond loop. Returns a builder so
    /// you can configure and either `.await` it or attach a
    /// CancellationToken first.
    ///
    /// The handler is `FnMut(Invocation) -> Future<Output = Result<HandlerResult, _>>`.
    /// Errors raised by the handler - both Err returns and panics
    /// turned into errors - are reported as `failed` invocations and
    /// the loop continues.
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
    cancel: Option<CancellationToken>,
    _phantom: std::marker::PhantomData<fn() -> (Fut, E)>,
}

impl<'a, F, Fut, E> ServeBuilder<'a, F, Fut, E>
where
    F: FnMut(Invocation) -> Fut + Send + 'a,
    Fut: Future<Output = std::result::Result<HandlerResult, E>> + Send + 'a,
    E: std::fmt::Display + Send + 'a,
{
    pub fn poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }
    pub fn batch_size(mut self, n: u32) -> Self {
        self.batch_size = n;
        self
    }
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Run the loop. Returns when the cancellation token fires.
    pub async fn run(mut self) -> Result<()> {
        let cancel = self.cancel.unwrap_or_default();
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let batch = match tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                r = self.inbox.pull(&self.agent_id, Some(self.batch_size)) => r,
            } {
                Ok(b) => b,
                Err(_) => {
                    // Transient pull errors - sleep and retry.
                    if cancel
                        .run_until_cancelled(tokio::time::sleep(self.poll_interval))
                        .await
                        .is_none()
                    {
                        return Ok(());
                    }
                    continue;
                }
            };
            if batch.is_empty() {
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

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
