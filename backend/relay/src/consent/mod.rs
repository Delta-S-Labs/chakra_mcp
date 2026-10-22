//! Owner consent (docs/superpowers/specs/2026-10-23-owner-consent-design.md).
//!
//! Gate 1: a capability can ask its owner before requests reach the agent.
//! A request to it is held (`awaiting_consent`) until an approver, signed in
//! to the web app, approves it as proposed or narrowed, or denies it. A
//! standing consent can cover later requests for a while. Approved requests
//! are delivered with a signed consent token bound to their input.
//!
//! - [`terms`]: what's proposed and what may be approved.
//! - [`enqueue`]: holding a request, or letting a consent cover it.
//! - [`approvers`]: who may decide.
//! - [`visibility`]: hiding held input from the target.
//! - [`deliver`] and [`token`]: tokens minted at delivery.
//! - [`expiry`]: the worker that expires undecided requests.
//! - [`cascade`]: cancelling requests when what they depend on is deleted.

pub mod approvers;
pub mod cascade;
pub mod deliver;
pub mod enqueue;
pub mod expiry;
pub mod metrics;
pub mod signer;
pub mod terms;
pub mod token;
pub mod visibility;

#[cfg(test)]
mod tests;
