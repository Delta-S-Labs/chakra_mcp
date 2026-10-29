//! Claims that go stale and calls that run out of time (waking pull agents
//! §9, §10).
//!
//! A claim holds a call for a lease (`CLAIM_LEASE_SECS`, or the pull's
//! `lease_secs`); every call has a deadline (`INVOCATION_DEFAULT_TTL`, or
//! `INVOCATION_HITL_TTL` for `human_in_loop`, or the caller's shorter
//! `expires_in`). Every 30 s each relay process sweeps, in batches with
//! `SKIP LOCKED`:
//!
//! 1. open calls from before the deadlines existed get one;
//! 2. open calls past their deadline time out, and the called agent gets an
//!    `invocation_expired` notice;
//! 3. claims past their lease go back to `pending`, or time out after
//!    [`MAX_DELIVERIES`] claims.
//!
//! The invoke path only computes a deadline in memory and writes it with the
//! row it already inserts.

use std::time::Duration;

use anyhow::bail;
use chakramcp_shared::error::ApiError;
use chakramcp_shared::telemetry::names::INVOCATION_EXPIRED_TOTAL;
use chakramcp_shared::wake::{Feed, Wake};
use metrics::counter;
use sqlx::PgPool;
use uuid::Uuid;

/// How often each process sweeps.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(30);
/// The longest lease a pull may ask for.
pub const MAX_LEASE_SECS: u32 = 86_400;
/// The shortest `expires_in` a caller may set.
pub const MIN_EXPIRES_IN_SECS: i64 = 10;
/// A call whose lease lapses this many times times out.
pub const MAX_DELIVERIES: i32 = 3;
/// Rows each pass handles at most.
const BATCH: i64 = 500;
/// Rows the backfill gives a deadline per pass.
const BACKFILL_BATCH: i64 = 1000;

const LEASE_RANGE: std::ops::RangeInclusive<u32> = 30..=MAX_LEASE_SECS;
const TTL_RANGE: std::ops::RangeInclusive<i64> = 60..=30 * 86_400;

/// Leases and deadlines (`CLAIM_LEASE_SECS`, `INVOCATION_*_TTL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvocationSettings {
    /// A claim's lease when the pull doesn't ask for one.
    pub lease_secs: u32,
    /// A call's deadline when the caller doesn't set a shorter one.
    pub default_ttl_secs: i64,
    /// The same, for a `human_in_loop` capability.
    pub hitl_ttl_secs: i64,
}

impl Default for InvocationSettings {
    fn default() -> Self {
        Self {
            lease_secs: 600,
            default_ttl_secs: 86_400,
            hitl_ttl_secs: 7 * 86_400,
        }
    }
}

impl InvocationSettings {
    pub fn from_sources(env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let set = |key: &str| env(key).filter(|v| !v.trim().is_empty());
        let lease_secs = match set("CLAIM_LEASE_SECS") {
            Some(raw) => match raw.trim().parse::<u32>() {
                Ok(n) if LEASE_RANGE.contains(&n) => n,
                _ => bail!(
                    "CLAIM_LEASE_SECS={raw:?} must be a whole number from {} to {}",
                    LEASE_RANGE.start(),
                    LEASE_RANGE.end()
                ),
            },
            None => defaults.lease_secs,
        };
        let ttl = |key: &str, default: i64| match set(key) {
            Some(raw) => parse_ttl(&raw).ok_or_else(|| {
                anyhow::anyhow!("{key}={raw:?} must be <n>m, <n>h or <n>d, from 1m to 30d")
            }),
            None => Ok(default),
        };
        Ok(Self {
            lease_secs,
            default_ttl_secs: ttl("INVOCATION_DEFAULT_TTL", defaults.default_ttl_secs)?,
            hitl_ttl_secs: ttl("INVOCATION_HITL_TTL", defaults.hitl_ttl_secs)?,
        })
    }

    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_sources(|key| std::env::var(key).ok())
    }

    /// For the startup log.
    pub fn summary(&self) -> String {
        format!(
            "claims lease for {}s; calls expire after {}s ({}s for human_in_loop)",
            self.lease_secs, self.default_ttl_secs, self.hitl_ttl_secs
        )
    }

    /// The lease for a pull that asked for `asked` seconds: clamped to a
    /// day, the default when it didn't ask.
    pub fn lease(&self, asked: Option<u32>) -> u32 {
        match asked {
            Some(n) if n > 0 => n.min(MAX_LEASE_SECS),
            _ => self.lease_secs,
        }
    }

    /// A new call's time to live, in seconds: the caller's `expires_in`,
    /// clamped to the default for its kind of capability. Under
    /// [`MIN_EXPIRES_IN_SECS`] is refused.
    pub fn ttl(&self, human_in_loop: bool, expires_in: Option<i64>) -> Result<i64, ApiError> {
        let default = if human_in_loop {
            self.hitl_ttl_secs
        } else {
            self.default_ttl_secs
        };
        match expires_in {
            None => Ok(default),
            Some(n) if n < MIN_EXPIRES_IN_SECS => Err(ApiError::InvalidRequest(format!(
                "expires_in must be at least {MIN_EXPIRES_IN_SECS} seconds"
            ))),
            Some(n) => Ok(n.min(default)),
        }
    }
}

/// `<n>m`, `<n>h` or `<n>d` → seconds, within 1 minute to 30 days.
fn parse_ttl(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (num, unit) = raw.split_at(raw.len().checked_sub(1)?);
    let n: i64 = num.parse().ok()?;
    let secs = n.checked_mul(match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return None,
    })?;
    TTL_RANGE.contains(&secs).then_some(secs)
}

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Swept {
    /// Open calls from before deadlines existed, given one.
    pub backfilled: u64,
    /// Never claimed before their deadline.
    pub not_picked_up: u64,
    /// Claimed and not answered before their deadline.
    pub not_answered: u64,
    /// Lease lapsed: back to `pending`.
    pub pended: u64,
    /// Lease lapsed for the last time: timed out.
    pub lease_exhausted: u64,
}

/// Sweep once. Each pass commits on its own, so a failure in one leaves
/// the others' work done.
pub async fn sweep_once(
    db: &PgPool,
    wake: &Wake,
    settings: &InvocationSettings,
) -> Result<Swept, sqlx::Error> {
    let mut swept = Swept {
        backfilled: backfill(db, settings).await?,
        ..Swept::default()
    };
    (swept.not_picked_up, swept.not_answered) = deadlines(db, wake).await?;
    (swept.pended, swept.lease_exhausted) = leases(db, wake).await?;
    for (reason, n) in [
        ("not_picked_up", swept.not_picked_up),
        ("not_answered", swept.not_answered),
        ("lease_exhausted", swept.lease_exhausted),
    ] {
        if n > 0 {
            counter!(INVOCATION_EXPIRED_TOTAL, "reason" => reason).increment(n);
        }
    }
    Ok(swept)
}

/// Give open calls without a deadline (from before migration 0053) a full
/// one from now, so calls already waiting get time to be answered.
/// `awaiting_consent` calls get theirs once approval makes them `pending`.
async fn backfill(db: &PgPool, settings: &InvocationSettings) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query!(
        r#"
        WITH due AS (
            SELECT i.id, c.semantics
              FROM relay_invocations i
              LEFT JOIN agent_capabilities c ON c.id = i.capability_id
             WHERE i.status IN ('pending', 'in_progress') AND i.expires_at IS NULL
             LIMIT $1
               FOR UPDATE OF i SKIP LOCKED
        )
        UPDATE relay_invocations i
           SET expires_at = now() + (CASE WHEN due.semantics = 'human_in_loop'
                                          THEN $3::bigint ELSE $2::bigint END)
                                    * interval '1 second'
          FROM due
         WHERE i.id = due.id
        "#,
        BACKFILL_BATCH,
        settings.default_ttl_secs,
        settings.hitl_ttl_secs,
    )
    .execute(db)
    .await?
    .rows_affected())
}

/// Time out open calls past their deadline. Returns how many were never
/// claimed, and how many were claimed and not answered.
///
/// Locks are taken in the reply-approval path's order, the call's pending
/// reply draft before the call, and always with `SKIP LOCKED`, so the sweep
/// never waits: a call whose draft is busy is left for the next pass. A
/// draft stored after the calls are locked stays pending until the consent
/// expiry worker expires it; its call is settled by then.
async fn deadlines(db: &PgPool, wake: &Wake) -> Result<(u64, u64), sqlx::Error> {
    let mut tx = db.begin().await?;
    let drafts = sqlx::query_scalar!(
        r#"
        SELECT cr.id
          FROM consent_requests cr
          JOIN relay_invocations i ON i.id = cr.invocation_id
         WHERE cr.kind = 'reply' AND cr.status = 'pending'
           AND i.status IN ('pending', 'in_progress') AND i.expires_at < now()
         LIMIT $1
           FOR UPDATE OF cr SKIP LOCKED
        "#,
        BATCH,
    )
    .fetch_all(&mut *tx)
    .await?;
    let timed_out = sqlx::query!(
        r#"
        WITH due AS (
            SELECT i.id
              FROM relay_invocations i
             WHERE i.status IN ('pending', 'in_progress') AND i.expires_at < now()
               AND NOT EXISTS (
                   SELECT 1 FROM consent_requests cr
                    WHERE cr.invocation_id = i.id AND cr.kind = 'reply'
                      AND cr.status = 'pending' AND cr.id <> ALL($2))
             ORDER BY i.expires_at
             LIMIT $1
               FOR UPDATE OF i SKIP LOCKED
        )
        UPDATE relay_invocations i
           SET status = 'timeout',
               error_message = CASE WHEN i.claimed_at IS NOT NULL
                                    THEN 'picked up, never answered'
                                    ELSE 'not picked up before its deadline' END,
               elapsed_ms = LEAST(EXTRACT(EPOCH FROM (now() - i.created_at)) * 1000,
                                  2147483647)::int,
               lease_until = NULL
          FROM due
         WHERE i.id = due.id
        RETURNING i.id, i.granter_agent_id, (i.claimed_at IS NOT NULL) AS "claimed!"
        "#,
        BATCH,
        &drafts,
    )
    .fetch_all(&mut *tx)
    .await?;
    if timed_out.is_empty() {
        return Ok((0, 0));
    }
    let ids: Vec<Uuid> = timed_out.iter().map(|r| r.id).collect();
    sqlx::query!(
        r#"
        UPDATE consent_requests SET status = 'cancelled'
         WHERE id = ANY($1) AND invocation_id = ANY($2)
        "#,
        &drafts,
        &ids,
    )
    .execute(&mut *tx)
    .await?;
    let told = tell_expired(
        &mut tx,
        timed_out
            .iter()
            .filter_map(|r| Some((r.granter_agent_id?, r.id))),
    )
    .await?;
    tx.commit().await?;
    for agent in told {
        wake.notify(agent, Feed::Notices);
    }
    let answered = timed_out.iter().filter(|r| r.claimed).count() as u64;
    Ok((timed_out.len() as u64 - answered, answered))
}

/// Hand claims past their lease back: `pending` again, with the agent woken,
/// or `timeout` once claimed [`MAX_DELIVERIES`] times. Returns how many of
/// each.
async fn leases(db: &PgPool, wake: &Wake) -> Result<(u64, u64), sqlx::Error> {
    let mut tx = db.begin().await?;
    let lapsed = sqlx::query!(
        r#"
        WITH due AS (
            SELECT id FROM relay_invocations
             WHERE status = 'in_progress' AND lease_until < now()
             ORDER BY lease_until
             LIMIT $1
               FOR UPDATE SKIP LOCKED
        )
        UPDATE relay_invocations i
           SET status = CASE WHEN i.deliveries >= $2 THEN 'timeout' ELSE 'pending' END,
               error_message = CASE WHEN i.deliveries >= $2 THEN $3 ELSE i.error_message END,
               elapsed_ms = CASE WHEN i.deliveries >= $2
                                 THEN LEAST(EXTRACT(EPOCH FROM (now() - i.created_at)) * 1000,
                                            2147483647)::int
                                 ELSE i.elapsed_ms END,
               claimed_at = CASE WHEN i.deliveries >= $2 THEN i.claimed_at END,
               claimed_by_user_id = CASE WHEN i.deliveries >= $2 THEN i.claimed_by_user_id END,
               lease_until = NULL
          FROM due
         WHERE i.id = due.id
        RETURNING i.id, i.granter_agent_id, i.status
        "#,
        BATCH,
        MAX_DELIVERIES,
        format!("claimed {MAX_DELIVERIES} times, never answered"),
    )
    .fetch_all(&mut *tx)
    .await?;
    if lapsed.is_empty() {
        return Ok((0, 0));
    }
    let told = tell_expired(
        &mut tx,
        lapsed
            .iter()
            .filter(|r| r.status == "timeout")
            .filter_map(|r| Some((r.granter_agent_id?, r.id))),
    )
    .await?;
    tx.commit().await?;
    for agent in told {
        wake.notify(agent, Feed::Notices);
    }
    let mut pended = 0;
    for r in lapsed.iter().filter(|r| r.status == "pending") {
        pended += 1;
        if let Some(agent) = r.granter_agent_id {
            wake.notify(agent, Feed::Invocations);
        }
    }
    Ok((pended, lapsed.len() as u64 - pended))
}

/// An `invocation_expired` notice to each `(agent, invocation)`, so a late
/// worker can stop. Returns the agents told, to wake after the commit.
async fn tell_expired(
    tx: &mut sqlx::PgConnection,
    expired: impl Iterator<Item = (Uuid, Uuid)>,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let (agents, invocations): (Vec<Uuid>, Vec<Uuid>) = expired.unzip();
    if agents.is_empty() {
        return Ok(agents);
    }
    let ids: Vec<Uuid> = agents.iter().map(|_| Uuid::now_v7()).collect();
    sqlx::query!(
        r#"
        INSERT INTO agent_notices (id, agent_id, kind, payload)
        SELECT n.id, n.agent, 'invocation_expired', jsonb_build_object('invocation_id', n.inv)
          FROM unnest($1::uuid[], $2::uuid[], $3::uuid[]) AS n(id, agent, inv)
        "#,
        &ids,
        &agents,
        &invocations,
    )
    .execute(tx)
    .await?;
    Ok(agents)
}

/// Sweep every [`SWEEP_INTERVAL`] for the life of the process.
pub fn spawn(db: PgPool, wake: Wake, settings: InvocationSettings) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(SWEEP_INTERVAL);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            match sweep_once(&db, &wake, &settings).await {
                Ok(swept) if swept != Swept::default() => {
                    tracing::info!(?swept, "invocation sweep");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "invocation sweep failed; will retry"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_their_defaults_and_ranges() {
        let none = InvocationSettings::from_sources(|_| None).unwrap();
        assert_eq!(none, InvocationSettings::default());
        let set = |pairs: &'static [(&'static str, &'static str)]| {
            InvocationSettings::from_sources(move |key| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (*v).to_owned())
            })
        };
        assert_eq!(
            set(&[
                ("CLAIM_LEASE_SECS", "120"),
                ("INVOCATION_DEFAULT_TTL", "2h"),
                ("INVOCATION_HITL_TTL", "30d"),
            ])
            .unwrap(),
            InvocationSettings {
                lease_secs: 120,
                default_ttl_secs: 7200,
                hitl_ttl_secs: 30 * 86_400,
            }
        );
        assert_eq!(
            set(&[("CLAIM_LEASE_SECS", "")]).unwrap(),
            InvocationSettings::default(),
            "empty is unset"
        );
        for bad in [
            ("CLAIM_LEASE_SECS", "10"),
            ("CLAIM_LEASE_SECS", "90000"),
            ("CLAIM_LEASE_SECS", "ten"),
            ("INVOCATION_DEFAULT_TTL", "30s"),
            ("INVOCATION_DEFAULT_TTL", "31d"),
            ("INVOCATION_HITL_TTL", "7"),
            ("INVOCATION_HITL_TTL", "d"),
        ] {
            let pairs: &'static [(&str, &str)] = Box::leak(Box::new([bad]));
            assert!(set(pairs).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn leases_and_ttls_are_clamped() {
        let s = InvocationSettings::default();
        assert_eq!(s.lease(None), 600);
        assert_eq!(s.lease(Some(0)), 600);
        assert_eq!(s.lease(Some(45)), 45);
        assert_eq!(s.lease(Some(1_000_000)), MAX_LEASE_SECS);

        assert_eq!(s.ttl(false, None).unwrap(), 86_400);
        assert_eq!(s.ttl(true, None).unwrap(), 7 * 86_400);
        assert_eq!(s.ttl(false, Some(60)).unwrap(), 60);
        assert_eq!(s.ttl(false, Some(10)).unwrap(), 10);
        assert_eq!(s.ttl(false, Some(10 * 86_400)).unwrap(), 86_400, "clamped");
        assert_eq!(s.ttl(true, Some(3 * 86_400)).unwrap(), 3 * 86_400);
        assert!(s.ttl(false, Some(9)).is_err());
        assert!(s.ttl(false, Some(-5)).is_err());
    }
}
