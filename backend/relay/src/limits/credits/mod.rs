//! Credits: per-account balances in milli-credits, drawn down by accepted
//! invocations and topped up by a monthly free grant (and, later,
//! purchases and admin grants).
//!
//! Design: `docs/specs/2026-09-23-credit-ledger-foundation-design.md`.
//!
//! Accounting never slows an invocation. The invocation path writes one row
//! to `credit_charge_queue` (in the same statement as its
//! `relay_invocations` row) and reads the in-memory switches in
//! [`CreditCache`]. The [`worker`] does everything else: charges the queue,
//! applies grants, and refreshes the switches every few seconds. A balance
//! can therefore overflow slightly negative between ticks — by design.

mod cache;
mod config;
pub mod worker;

pub use cache::CreditCache;
pub use config::CreditsConfig;
pub use worker::spawn_worker;

#[cfg(test)]
pub(crate) use cache::exhausted_cache;
