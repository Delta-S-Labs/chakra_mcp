//! Owner consent decisions, shared by the relay's approvals API and the
//! app's built-in approval page (self-hosted servers have no web app).
//! Spec: docs/superpowers/specs/2026-10-23-owner-consent-design.md.
//!
//! - [`terms`]: what a requester proposes and what may be approved.
//! - [`approvers`]: who may decide.
//! - [`decide`]: viewing, approving, denying and revoking.
//!
//! Holding requests, tokens and delivery stay in the relay
//! (`chakramcp_relay::consent`).

pub mod approvers;
pub mod decide;
pub mod metrics;
pub mod terms;
