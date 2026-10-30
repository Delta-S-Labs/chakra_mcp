//! Waking pull agents (design: `docs/superpowers/specs/2026-10-28-waking-pull-agents-design.md`
//! §4): an in-memory signal per agent and feed.
//!
//! Wherever work is queued for a pull agent, the code calls
//! [`Wake::notify`] after its commit. That bumps a generation counter for the
//! agent's feed, which wakes any pull waiting on it in this process, and
//! queues the key for [`bus`], which tells the other processes through
//! Postgres `LISTEN/NOTIFY`. `notify` is a plain function: it takes two
//! short locks and does no I/O, so it's safe on the invocation path.
//!
//! A waiting pull calls [`Wake::try_wait`] before its first claim. The
//! generation is marked as seen at that moment, so work queued between the
//! claim and the wait still wakes it.

pub mod bus;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::bail;
use metrics::{counter, gauge};
use tokio::sync::{watch, Notify};
use tokio::time::Instant;
use uuid::Uuid;

use crate::hosting::env_value;
use crate::telemetry::names;

/// One of a pull agent's three inbox feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feed {
    /// `/v1/inbox`: invocations waiting to be claimed.
    Invocations,
    /// `/v1/inbox/notices`.
    Notices,
    /// `/v1/inbox/calls`: call events delivered to the agent.
    Calls,
}

impl Feed {
    /// The feed's code in [`bus`] payloads.
    pub fn code(self) -> &'static str {
        match self {
            Feed::Invocations => "i",
            Feed::Notices => "n",
            Feed::Calls => "c",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "i" => Some(Feed::Invocations),
            "n" => Some(Feed::Notices),
            "c" => Some(Feed::Calls),
            _ => None,
        }
    }
}

type Key = (Uuid, Feed);

/// How long pulls may wait, and how many at once (`INBOX_MAX_WAIT_SECS`,
/// `INBOX_MAX_WAITERS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeSettings {
    /// The longest `wait` a pull is given, in seconds. 0 turns waiting off.
    pub max_wait_secs: u32,
    /// Waiting pulls per process. Past it, a pull answers at once.
    pub max_waiters: usize,
}

impl Default for WakeSettings {
    fn default() -> Self {
        Self {
            max_wait_secs: 25,
            max_waiters: 2000,
        }
    }
}

/// The most `INBOX_MAX_WAIT_SECS` may be: common proxies cut a request off
/// at 60 seconds.
const MAX_WAIT_CAP: u32 = 60;

impl WakeSettings {
    /// From the given environment. An empty value counts as unset; a bad one
    /// is an error, so a typo stops startup instead of being ignored.
    pub fn from_sources(env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let max_wait_secs = match env_value(&env, "INBOX_MAX_WAIT_SECS") {
            Some(raw) => match raw.parse::<u32>() {
                Ok(n) if n <= MAX_WAIT_CAP => n,
                _ => bail!(
                    "INBOX_MAX_WAIT_SECS={raw:?} must be a whole number from 0 to {MAX_WAIT_CAP}"
                ),
            },
            None => defaults.max_wait_secs,
        };
        let max_waiters = match env_value(&env, "INBOX_MAX_WAITERS") {
            Some(raw) => raw
                .parse::<usize>()
                .map_err(|_| anyhow::anyhow!("INBOX_MAX_WAITERS={raw:?} must be a whole number"))?,
            None => defaults.max_waiters,
        };
        Ok(Self {
            max_wait_secs,
            max_waiters,
        })
    }

    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok())
    }

    /// For the startup log.
    pub fn summary(&self) -> String {
        if self.max_wait_secs == 0 || self.max_waiters == 0 {
            "inbox pulls don't wait".to_owned()
        } else {
            format!(
                "inbox pulls wait up to {}s, {} at a time",
                self.max_wait_secs, self.max_waiters
            )
        }
    }
}

/// The hub. Cheap to clone; every clone is the same hub.
#[derive(Clone)]
pub struct Wake(Arc<Inner>);

struct Inner {
    /// This process, in [`bus`] payloads, so it can skip its own messages.
    process: Uuid,
    /// The generation of every key a pull is waiting on. A key nobody waits
    /// on has no entry, so notifying it costs one lookup.
    keys: Mutex<HashMap<Key, watch::Sender<u64>>>,
    /// Keys notified here and not yet sent to the other processes.
    outbound: Mutex<HashSet<Key>>,
    outbound_ready: Notify,
    waiters: AtomicUsize,
    max_waiters: usize,
    closed: watch::Sender<bool>,
    /// A wake-URL dispatcher runs here (design §7): notified agents are
    /// gathered for it, and other processes' wakes marked `d` too.
    dispatching: AtomicBool,
    dispatch: Mutex<HashSet<Uuid>>,
    dispatch_ready: Notify,
}

impl std::fmt::Debug for Wake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wake")
            .field("process", &self.0.process)
            .field("waiters", &self.0.waiters.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for Wake {
    fn default() -> Self {
        Self::new(WakeSettings::default().max_waiters)
    }
}

impl Wake {
    pub fn new(max_waiters: usize) -> Self {
        Self(Arc::new(Inner {
            process: Uuid::now_v7(),
            keys: Mutex::new(HashMap::new()),
            outbound: Mutex::new(HashSet::new()),
            outbound_ready: Notify::new(),
            waiters: AtomicUsize::new(0),
            max_waiters,
            closed: watch::Sender::new(false),
            dispatching: AtomicBool::new(false),
            dispatch: Mutex::new(HashSet::new()),
            dispatch_ready: Notify::new(),
        }))
    }

    /// Work was queued for `agent`'s `feed`: wake its waiting pulls here and,
    /// through [`bus`], in every other process. Call it after the commit that
    /// queued the work. No I/O.
    pub fn notify(&self, agent: Uuid, feed: Feed) {
        self.signal_local(agent, feed);
        lock(&self.0.outbound).insert((agent, feed));
        self.0.outbound_ready.notify_one();
        self.queue_dispatch(agent);
        counter!(names::WAKE_NOTIFY_TOTAL).increment(1);
    }

    /// This process runs a wake-URL dispatcher: gather notified agents for
    /// it from now on, and other processes' wakes marked `d`.
    pub fn start_dispatching(&self) {
        self.0.dispatching.store(true, Ordering::Relaxed);
    }

    /// Whether a wake-URL dispatcher runs here.
    pub fn dispatching(&self) -> bool {
        self.0.dispatching.load(Ordering::Relaxed)
    }

    /// Hand `agent` to this process's dispatcher, if one runs. No I/O.
    fn queue_dispatch(&self, agent: Uuid) {
        if self.dispatching() {
            lock(&self.0.dispatch).insert(agent);
            self.0.dispatch_ready.notify_one();
        }
    }

    /// Wait until agents are gathered for the dispatcher.
    pub async fn dispatch_ready(&self) {
        self.0.dispatch_ready.notified().await;
    }

    /// Take the agents gathered for the dispatcher.
    pub fn take_dispatch(&self) -> Vec<Uuid> {
        lock(&self.0.dispatch).drain().collect()
    }

    /// Wake `agent`'s pulls on `feed` in this process only. For wakes that
    /// came from another process.
    pub fn signal_local(&self, agent: Uuid, feed: Feed) {
        if let Some(sender) = lock(&self.0.keys).get(&(agent, feed)) {
            sender.send_modify(|generation| *generation = generation.wrapping_add(1));
        }
    }

    /// Every key a pull is waiting on in this process.
    pub fn waited_keys(&self) -> Vec<(Uuid, Feed)> {
        lock(&self.0.keys).keys().copied().collect()
    }

    /// Start waiting on `agent`'s `feed`, before the first claim. `None`
    /// when every slot is taken or the hub is closed: answer at once.
    pub fn try_wait(&self, agent: Uuid, feed: Feed) -> Option<Waiter> {
        if *self.0.closed.borrow() {
            return None;
        }
        let taken = self
            .0
            .waiters
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.0.max_waiters).then_some(n + 1)
            });
        let Ok(previous) = taken else {
            return None;
        };
        gauge!(names::INBOX_WAITERS).set((previous + 1) as f64);
        let changes = {
            let mut keys = lock(&self.0.keys);
            let sender = keys
                .entry((agent, feed))
                .or_insert_with(|| watch::Sender::new(0));
            let mut rx = sender.subscribe();
            rx.mark_unchanged();
            rx
        };
        let mut closed = self.0.closed.subscribe();
        closed.mark_unchanged();
        Some(Waiter {
            wake: self.clone(),
            key: (agent, feed),
            changes: Some(changes),
            closed,
        })
    }

    /// Shutting down: every waiting pull returns now, and new ones don't wait.
    pub fn close(&self) {
        self.0.closed.send_replace(true);
    }

    pub fn is_closed(&self) -> bool {
        *self.0.closed.borrow()
    }

    /// Waiting pulls in this process.
    pub fn waiting(&self) -> usize {
        self.0.waiters.load(Ordering::Acquire)
    }

    /// Take the keys queued for the other processes.
    fn drain_outbound(&self) -> Vec<Key> {
        lock(&self.0.outbound).drain().collect()
    }

    /// Put keys back after a failed send.
    fn requeue(&self, keys: impl IntoIterator<Item = Key>) {
        lock(&self.0.outbound).extend(keys);
    }

    fn process(&self) -> Uuid {
        self.0.process
    }
}

/// Why a [`Waiter`] stopped waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Woke {
    /// New work may be there: claim again.
    Changed,
    /// The wait is over.
    Deadline,
    /// The process is shutting down: return without claiming.
    Closed,
}

/// A pull waiting on one agent's feed. Dropping it frees the slot.
pub struct Waiter {
    wake: Wake,
    key: Key,
    changes: Option<watch::Receiver<u64>>,
    closed: watch::Receiver<bool>,
}

impl Waiter {
    /// Wait for new work, the deadline, or shutdown, whichever comes first.
    pub async fn changed(&mut self, until: Instant) -> Woke {
        let Some(changes) = self.changes.as_mut() else {
            return Woke::Closed;
        };
        if *self.closed.borrow() {
            return Woke::Closed;
        }
        tokio::select! {
            changed = changes.changed() => match changed {
                Ok(()) => Woke::Changed,
                // The sender lives in the map while this receiver does.
                Err(_) => Woke::Closed,
            },
            _ = self.closed.changed() => Woke::Closed,
            _ = tokio::time::sleep_until(until) => Woke::Deadline,
        }
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        // Drop the receiver first, then remove the key if no one else
        // waits on it.
        self.changes = None;
        {
            let mut keys = lock(&self.wake.0.keys);
            if keys
                .get(&self.key)
                .is_some_and(|sender| sender.receiver_count() == 0)
            {
                keys.remove(&self.key);
            }
        }
        let left = self.wake.0.waiters.fetch_sub(1, Ordering::AcqRel) - 1;
        gauge!(names::INBOX_WAITERS).set(left as f64);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(2)
    }

    #[tokio::test]
    async fn a_notify_wakes_a_waiter() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        let mut waiter = wake.try_wait(agent, Feed::Invocations).unwrap();
        let notifier = wake.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            notifier.notify(agent, Feed::Invocations);
        });
        assert_eq!(waiter.changed(soon()).await, Woke::Changed);
    }

    #[tokio::test]
    async fn work_between_the_claim_and_the_wait_is_seen() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        let mut waiter = wake.try_wait(agent, Feed::Calls).unwrap();
        // The claim found nothing; the work lands before the wait starts.
        wake.notify(agent, Feed::Calls);
        assert_eq!(waiter.changed(soon()).await, Woke::Changed);
    }

    #[tokio::test]
    async fn other_feeds_and_agents_dont_wake_it() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        let mut waiter = wake.try_wait(agent, Feed::Notices).unwrap();
        wake.notify(agent, Feed::Invocations);
        wake.notify(Uuid::now_v7(), Feed::Notices);
        let until = Instant::now() + Duration::from_millis(50);
        assert_eq!(waiter.changed(until).await, Woke::Deadline);
    }

    #[tokio::test]
    async fn notifying_a_key_nobody_waits_on_stores_nothing() {
        let wake = Wake::new(10);
        wake.notify(Uuid::now_v7(), Feed::Invocations);
        assert!(wake.waited_keys().is_empty());
    }

    #[tokio::test]
    async fn the_cap_holds_and_a_dropped_waiter_frees_its_slot() {
        let wake = Wake::new(1);
        let agent = Uuid::now_v7();
        let first = wake.try_wait(agent, Feed::Invocations).unwrap();
        assert!(wake.try_wait(agent, Feed::Invocations).is_none());
        drop(first);
        assert_eq!(wake.waiting(), 0);
        assert!(wake.waited_keys().is_empty());
        assert!(wake.try_wait(agent, Feed::Invocations).is_some());
    }

    #[tokio::test]
    async fn a_key_stays_while_someone_still_waits_on_it() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        let first = wake.try_wait(agent, Feed::Invocations).unwrap();
        let mut second = wake.try_wait(agent, Feed::Invocations).unwrap();
        drop(first);
        assert_eq!(wake.waited_keys(), vec![(agent, Feed::Invocations)]);
        wake.notify(agent, Feed::Invocations);
        assert_eq!(second.changed(soon()).await, Woke::Changed);
    }

    #[tokio::test]
    async fn close_releases_waiters_and_refuses_new_ones() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        let mut waiter = wake.try_wait(agent, Feed::Invocations).unwrap();
        let closer = wake.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            closer.close();
        });
        assert_eq!(waiter.changed(soon()).await, Woke::Closed);
        assert!(wake.try_wait(agent, Feed::Invocations).is_none());
    }

    #[tokio::test]
    async fn a_burst_leaves_one_outbound_entry() {
        let wake = Wake::new(10);
        let agent = Uuid::now_v7();
        for _ in 0..3 {
            wake.notify(agent, Feed::Calls);
        }
        wake.notify(agent, Feed::Notices);
        let mut sent = wake.drain_outbound();
        sent.sort_by_key(|(_, feed)| feed.code());
        assert_eq!(sent, vec![(agent, Feed::Calls), (agent, Feed::Notices)]);
        assert!(wake.drain_outbound().is_empty());
    }

    #[test]
    fn settings_default_and_parse() {
        let none = |_: &str| None;
        assert_eq!(
            WakeSettings::from_sources(none).unwrap(),
            WakeSettings::default()
        );
        let set = |key: &str| match key {
            "INBOX_MAX_WAIT_SECS" => Some("10".to_owned()),
            "INBOX_MAX_WAITERS" => Some(" ".to_owned()),
            _ => None,
        };
        let s = WakeSettings::from_sources(set).unwrap();
        assert_eq!(s.max_wait_secs, 10);
        assert_eq!(s.max_waiters, 2000);
        let bad = |key: &str| (key == "INBOX_MAX_WAIT_SECS").then(|| "90".to_owned());
        assert!(WakeSettings::from_sources(bad).is_err());
        let bad = |key: &str| (key == "INBOX_MAX_WAITERS").then(|| "lots".to_owned());
        assert!(WakeSettings::from_sources(bad).is_err());
    }
}
