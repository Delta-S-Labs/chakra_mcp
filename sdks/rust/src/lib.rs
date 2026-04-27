//! Rust SDK for the [ChakraMCP](https://chakramcp.com) relay.
//!
//! Async-only — the audience for this crate is tokio-based agent
//! runtimes. For sync usage, wrap calls in
//! [`tokio::runtime::Runtime::block_on`].
//!
//! ## Quick start
//!
//! ```no_run
//! # use chakramcp::ChakraMCP;
//! # async fn run() -> Result<(), chakramcp::Error> {
//! let chakra = ChakraMCP::new(std::env::var("CHAKRAMCP_API_KEY").unwrap())?;
//! let me = chakra.me().await?;
//! println!("hi {}", me.user.email);
//! # Ok(()) }
//! ```
//!
//! ## Inbox loop (the killer feature)
//!
//! ```ignore
//! use chakramcp::{ChakraMCP, HandlerResult};
//! use tokio_util::sync::CancellationToken;
//! use std::convert::Infallible;
//! use std::future::IntoFuture;
//!
//! # async fn run(chakra: ChakraMCP, agent_id: String) -> Result<(), chakramcp::Error> {
//! let cancel = CancellationToken::new();
//! chakra
//!     .inbox()
//!     .serve(&agent_id, |inv| async move {
//!         Ok::<_, Infallible>(HandlerResult::Succeeded(
//!             serde_json::json!({"echoed": inv.input_preview}),
//!         ))
//!     })
//!     .with_cancellation(cancel.clone())
//!     .into_future()
//!     .await?;
//! # Ok(()) }
//! ```

mod client;
mod error;
mod inbox;
mod resources;
mod types;

pub use client::{ChakraMCP, ChakraMCPBuilder, PollOpts};
pub use error::{Error, Result};
pub use inbox::{HandlerResult, InboxClient, ServeBuilder};
pub use resources::{
    AgentsClient, CapabilitiesClient, FriendshipsClient, GrantsClient, InvocationsClient,
};
pub use types::*;
