//! Calls client + the long-running `serve` loop.
//!
//! A call is a two-way session between two agents: messages back and
//! forth, and capability requests made in it (`InvokeRequest::call_id`).
//! The relay may keep the call's memory (`CallMemoryMode::Call`).

use std::any::Any;
use std::fmt::Display;
use std::future::{Future, IntoFuture};
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::time::Duration;

use futures::FutureExt;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::client::ChakraMCP;
use crate::error::{Error, Result};
use crate::resources::{serde_plain, urlencode};
use crate::types::{
    Call, CallEntry, CallEntryKind, CallFeedItem, CallHistoryQuery, CallList, CallMemory,
    CallRecall, ListCallsQuery, StartCallRequest,
};

pub struct CallsClient<'a> {
    parent: &'a ChakraMCP,
}

impl<'a> CallsClient<'a> {
    pub(crate) fn new(parent: &'a ChakraMCP) -> Self {
        Self { parent }
    }

    /// Invite another agent to a call. The call starts `Ringing`.
    pub async fn start(&self, body: &StartCallRequest) -> Result<Call> {
        self.parent.relay_post("/v1/calls", body).await
    }

    /// Your agents' calls, newest first, a page at a time.
    pub async fn list(&self, query: ListCallsQuery) -> Result<CallList> {
        let mut q = Vec::new();
        if let Some(a) = query.agent_id.as_deref() {
            q.push(format!("agent_id={}", urlencode(a)));
        }
        if let Some(s) = query.state {
            q.push(format!("state={}", serde_plain(&s)));
        }
        if let Some(n) = query.limit {
            q.push(format!("limit={n}"));
        }
        if let Some(c) = query.cursor.as_deref() {
            q.push(format!("cursor={}", urlencode(c)));
        }
        self.parent
            .relay_get(&format!("/v1/calls{}", query_string(&q)))
            .await
    }

    pub async fn get(&self, call_id: &str) -> Result<Call> {
        self.parent
            .relay_get(&format!("/v1/calls/{}", urlencode(call_id)))
            .await
    }

    /// Accept a ringing call as `agent_id`, with an optional first reply.
    pub async fn accept(
        &self,
        call_id: &str,
        agent_id: &str,
        message: Option<&str>,
    ) -> Result<Call> {
        #[derive(Serialize)]
        struct Body<'b> {
            agent_id: &'b str,
            #[serde(skip_serializing_if = "Option::is_none")]
            message: Option<&'b str>,
        }
        self.parent
            .relay_post(
                &format!("/v1/calls/{}/accept", urlencode(call_id)),
                &Body { agent_id, message },
            )
            .await
    }

    /// Decline a ringing call as `agent_id`, with an optional reason.
    pub async fn decline(
        &self,
        call_id: &str,
        agent_id: &str,
        reason: Option<&str>,
    ) -> Result<Call> {
        #[derive(Serialize)]
        struct Body<'b> {
            agent_id: &'b str,
            #[serde(skip_serializing_if = "Option::is_none")]
            reason: Option<&'b str>,
        }
        self.parent
            .relay_post(
                &format!("/v1/calls/{}/decline", urlencode(call_id)),
                &Body { agent_id, reason },
            )
            .await
    }

    /// End a ringing or active call as `agent_id`.
    pub async fn end(&self, call_id: &str, agent_id: &str) -> Result<Call> {
        #[derive(Serialize)]
        struct Body<'b> {
            agent_id: &'b str,
        }
        self.parent
            .relay_post(
                &format!("/v1/calls/{}/end", urlencode(call_id)),
                &Body { agent_id },
            )
            .await
    }

    /// Send a message on an active call as `agent_id`. `text` and `data`
    /// together are at most 16 KiB.
    pub async fn send(
        &self,
        call_id: &str,
        agent_id: &str,
        text: &str,
        data: Option<Value>,
    ) -> Result<CallEntry> {
        #[derive(Serialize)]
        struct Body<'b> {
            agent_id: &'b str,
            text: &'b str,
            #[serde(skip_serializing_if = "Option::is_none")]
            data: Option<Value>,
        }
        self.parent
            .relay_post(
                &format!("/v1/calls/{}/messages", urlencode(call_id)),
                &Body {
                    agent_id,
                    text,
                    data,
                },
            )
            .await
    }

    /// The call's stored transcript, until its content is deleted.
    pub async fn history(&self, call_id: &str, query: CallHistoryQuery) -> Result<Vec<CallEntry>> {
        let mut q = Vec::new();
        if let Some(after) = query.after {
            q.push(format!("after={after}"));
        }
        if let Some(n) = query.limit {
            q.push(format!("limit={n}"));
        }
        if let Some(text) = query.q.as_deref() {
            q.push(format!("q={}", urlencode(text)));
        }
        self.parent
            .relay_get(&format!(
                "/v1/calls/{}/messages{}",
                urlencode(call_id),
                query_string(&q)
            ))
            .await
    }

    /// The call's memory (`memory: Call` only; 503
    /// `chk.calls.memory_unavailable` otherwise).
    pub async fn memory(&self, call_id: &str) -> Result<CallMemory> {
        self.parent
            .relay_get(&format!("/v1/calls/{}/memory", urlencode(call_id)))
            .await
    }

    /// Ask the call's memory a question as `agent_id` (at most 1 KiB). It
    /// returns the messages that answer it; the agent answers for itself.
    pub async fn ask(&self, call_id: &str, agent_id: &str, question: &str) -> Result<CallRecall> {
        #[derive(Serialize)]
        struct Body<'b> {
            agent_id: &'b str,
            question: &'b str,
        }
        self.parent
            .relay_post(
                &format!("/v1/calls/{}/memory/ask", urlencode(call_id)),
                &Body { agent_id, question },
            )
            .await
    }

    /// Claim the agent's undelivered call entries, oldest first (at most
    /// 25). Each is delivered once.
    pub async fn pull(&self, agent_id: &str, limit: Option<u32>) -> Result<Vec<CallFeedItem>> {
        let mut path = format!("/v1/inbox/calls?agent_id={}", urlencode(agent_id));
        if let Some(n) = limit {
            path.push_str(&format!("&limit={n}"));
        }
        self.parent.relay_get(&path).await
    }

    /// Long-running loop for a pull agent: pull the calls feed, hand each
    /// item to a handler, and answer as `agent_id`. Returns a builder:
    /// set the handlers, then `.await` it.
    ///
    /// - An invite goes to [`CallsServeBuilder::on_invite`], which accepts
    ///   or declines it. Without one, every invite is accepted.
    /// - A message goes to [`CallsServeBuilder::on_message`]; its reply,
    ///   if any, is sent as the agent. Without one, messages are skipped.
    /// - `accepted`, `declined`, `ended` and `missed` go to
    ///   [`CallsServeBuilder::on_event`], if set.
    ///
    /// A handler that returns an error or panics, and an answer the relay
    /// refuses, go to [`CallsServeBuilder::on_error`] (stderr without
    /// one), and the loop carries on. An answer refused with
    /// `chk.calls.not_active` (the call ended meanwhile) is ignored.
    ///
    /// ```no_run
    /// use chakramcp::{CallReply, ChakraMCP, InviteAnswer};
    /// use std::convert::Infallible;
    /// use tokio_util::sync::CancellationToken;
    ///
    /// # async fn run(chakra: ChakraMCP, agent_id: String) -> Result<(), chakramcp::Error> {
    /// let cancel = CancellationToken::new();
    /// chakra
    ///     .calls()
    ///     .serve(&agent_id)
    ///     .on_invite(|_item| async { Ok::<_, Infallible>(InviteAnswer::accept_with("Hi!")) })
    ///     .on_message(|item| async move {
    ///         let text = item.entry.text.unwrap_or_default();
    ///         Ok::<_, Infallible>(Some(CallReply::text(format!("You said: {text}"))))
    ///     })
    ///     .with_cancellation(cancel.clone())
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn serve(self, agent_id: impl Into<String>) -> CallsServeBuilder<'a> {
        CallsServeBuilder {
            calls: self,
            agent_id: agent_id.into(),
            on_invite: None,
            on_message: None,
            on_event: None,
            on_error: None,
            poll_interval: Duration::from_secs(2),
            batch_size: 25,
            cancel: None,
        }
    }
}

/// A reply `on_message` sends on the call: text, and optional JSON data.
#[derive(Debug, Clone, PartialEq)]
pub struct CallReply {
    pub text: String,
    pub data: Option<Value>,
}

impl CallReply {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            data: None,
        }
    }

    pub fn with_data(text: impl Into<String>, data: Value) -> Self {
        Self {
            text: text.into(),
            data: Some(data),
        }
    }
}

impl From<&str> for CallReply {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

impl From<String> for CallReply {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

/// What `on_invite` does with an invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteAnswer {
    /// Accept, with an optional first reply.
    Accept { message: Option<String> },
    /// Decline, with an optional reason (at most 500 characters).
    Decline { reason: Option<String> },
}

impl InviteAnswer {
    pub fn accept() -> Self {
        Self::Accept { message: None }
    }
    pub fn accept_with(message: impl Into<String>) -> Self {
        Self::Accept {
            message: Some(message.into()),
        }
    }
    pub fn decline() -> Self {
        Self::Decline { reason: None }
    }
    pub fn decline_with(reason: impl Into<String>) -> Self {
        Self::Decline {
            reason: Some(reason.into()),
        }
    }
}

/// Something that went wrong in [`CallsClient::serve`]'s loop, which
/// carries on regardless.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallsServeError {
    /// Pulling the calls feed failed; the loop waits a poll interval and
    /// pulls again.
    #[error("pulling the calls feed failed: {0}")]
    Pull(#[source] Error),
    /// A handler returned an error or panicked.
    #[error("{handler} failed on call {call_id} (entry {seq}): {message}")]
    Handler {
        /// `on_invite`, `on_message` or `on_event`.
        handler: &'static str,
        call_id: String,
        seq: i64,
        message: String,
    },
    /// The relay refused the agent's answer: accepting or declining an
    /// invite, or sending a reply.
    #[error("answering call {call_id} (entry {seq}) failed: {error}")]
    Respond {
        call_id: String,
        seq: i64,
        #[source]
        error: Error,
    },
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type Handler<'a, T> =
    Box<dyn FnMut(CallFeedItem) -> BoxFuture<'a, std::result::Result<T, String>> + Send + 'a>;

fn boxed<'a, F, Fut, T, E>(mut f: F) -> Handler<'a, T>
where
    F: FnMut(CallFeedItem) -> Fut + Send + 'a,
    Fut: Future<Output = std::result::Result<T, E>> + Send + 'a,
    E: Display + Send + 'a,
{
    Box::new(move |item| {
        let fut = f(item);
        Box::pin(async move { fut.await.map_err(|e| e.to_string()) })
    })
}

/// Run a handler, turning a panic into an error.
async fn run_handler<T>(
    h: &mut Handler<'_, T>,
    item: CallFeedItem,
) -> std::result::Result<T, String> {
    let fut = std::panic::catch_unwind(AssertUnwindSafe(|| h(item))).map_err(panic_message)?;
    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .unwrap_or_else(|p| Err(panic_message(p)))
}

fn panic_message(p: Box<dyn Any + Send>) -> String {
    let msg = p
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into());
    format!("panicked: {msg}")
}

fn is_not_active(e: &Error) -> bool {
    matches!(e, Error::Api { code, .. } if code == "chk.calls.not_active")
}

pub struct CallsServeBuilder<'a> {
    calls: CallsClient<'a>,
    agent_id: String,
    on_invite: Option<Handler<'a, InviteAnswer>>,
    on_message: Option<Handler<'a, Option<CallReply>>>,
    on_event: Option<Handler<'a, ()>>,
    on_error: Option<Box<dyn FnMut(CallsServeError) + Send + 'a>>,
    poll_interval: Duration,
    batch_size: u32,
    cancel: Option<CancellationToken>,
}

impl<'a> CallsServeBuilder<'a> {
    /// Decide on each invite (the item's `entry.text` is the opening
    /// message). Without it, every invite is accepted. An error leaves the
    /// invite unanswered.
    pub fn on_invite<F, Fut, E>(mut self, f: F) -> Self
    where
        F: FnMut(CallFeedItem) -> Fut + Send + 'a,
        Fut: Future<Output = std::result::Result<InviteAnswer, E>> + Send + 'a,
        E: Display + Send + 'a,
    {
        self.on_invite = Some(boxed(f));
        self
    }

    /// Handle each message: the call, the entry and, in call-memory mode,
    /// the call's memory. `Ok(Some(reply))` sends it as the agent;
    /// `Ok(None)` sends nothing.
    pub fn on_message<F, Fut, E>(mut self, f: F) -> Self
    where
        F: FnMut(CallFeedItem) -> Fut + Send + 'a,
        Fut: Future<Output = std::result::Result<Option<CallReply>, E>> + Send + 'a,
        E: Display + Send + 'a,
    {
        self.on_message = Some(boxed(f));
        self
    }

    /// Handle `accepted`, `declined`, `ended` and `missed` entries.
    /// Without it, they're skipped.
    pub fn on_event<F, Fut, E>(mut self, f: F) -> Self
    where
        F: FnMut(CallFeedItem) -> Fut + Send + 'a,
        Fut: Future<Output = std::result::Result<(), E>> + Send + 'a,
        E: Display + Send + 'a,
    {
        self.on_event = Some(boxed(f));
        self
    }

    /// Where the loop's errors go. Without it, they're printed to stderr.
    pub fn on_error<F>(mut self, f: F) -> Self
    where
        F: FnMut(CallsServeError) + Send + 'a,
    {
        self.on_error = Some(Box::new(f));
        self
    }

    /// How long to wait after an empty pull. Default 2 s.
    pub fn poll_interval(mut self, d: Duration) -> Self {
        self.poll_interval = d;
        self
    }

    /// Items per pull, at most 25 (the default).
    pub fn batch_size(mut self, n: u32) -> Self {
        self.batch_size = n;
        self
    }

    /// Stop the loop when `token` fires. The feed delivers each entry
    /// once, so a pull in flight and the batch it returns are handled
    /// first; drop the future to stop at once.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Run the loop. Returns when the cancellation token fires.
    pub async fn run(mut self) -> Result<()> {
        let cancel = self.cancel.take().unwrap_or_default();
        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }
            let batch = match self.calls.pull(&self.agent_id, Some(self.batch_size)).await {
                Ok(b) => b,
                Err(e) => {
                    self.report(CallsServeError::Pull(e));
                    Vec::new()
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
            for item in batch {
                self.handle(item).await;
            }
        }
    }

    async fn handle(&mut self, item: CallFeedItem) {
        let call_id = item.call.id.clone();
        let seq = item.entry.seq;
        let handler_error = |handler, message| CallsServeError::Handler {
            handler,
            call_id: call_id.clone(),
            seq,
            message,
        };
        match item.entry.kind {
            CallEntryKind::Invite => {
                let answer = match self.on_invite.as_mut() {
                    None => InviteAnswer::accept(),
                    Some(h) => match run_handler(h, item).await {
                        Ok(answer) => answer,
                        Err(message) => return self.report(handler_error("on_invite", message)),
                    },
                };
                let sent = match answer {
                    InviteAnswer::Accept { message } => self
                        .calls
                        .accept(&call_id, &self.agent_id, message.as_deref())
                        .await
                        .map(drop),
                    InviteAnswer::Decline { reason } => self
                        .calls
                        .decline(&call_id, &self.agent_id, reason.as_deref())
                        .await
                        .map(drop),
                };
                self.answered(sent, &call_id, seq);
            }
            CallEntryKind::Message => {
                let Some(h) = self.on_message.as_mut() else {
                    return;
                };
                match run_handler(h, item).await {
                    Ok(Some(reply)) => {
                        let sent = self
                            .calls
                            .send(&call_id, &self.agent_id, &reply.text, reply.data)
                            .await
                            .map(drop);
                        self.answered(sent, &call_id, seq);
                    }
                    Ok(None) => {}
                    Err(message) => self.report(handler_error("on_message", message)),
                }
            }
            CallEntryKind::Accepted
            | CallEntryKind::Declined
            | CallEntryKind::Ended
            | CallEntryKind::Missed => {
                let Some(h) = self.on_event.as_mut() else {
                    return;
                };
                if let Err(message) = run_handler(h, item).await {
                    self.report(handler_error("on_event", message));
                }
            }
            _ => {}
        }
    }

    /// Report a refused answer, unless the call simply ended meanwhile.
    fn answered(&mut self, sent: Result<()>, call_id: &str, seq: i64) {
        match sent {
            Err(e) if !is_not_active(&e) => self.report(CallsServeError::Respond {
                call_id: call_id.to_string(),
                seq,
                error: e,
            }),
            _ => {}
        }
    }

    fn report(&mut self, e: CallsServeError) {
        match self.on_error.as_mut() {
            Some(f) => f(e),
            None => eprintln!("chakramcp calls.serve: {e}"),
        }
    }
}

impl<'a> IntoFuture for CallsServeBuilder<'a> {
    type Output = Result<()>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

fn query_string(q: &[String]) -> String {
    if q.is_empty() {
        String::new()
    } else {
        format!("?{}", q.join("&"))
    }
}
