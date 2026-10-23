//! System One compliance checks and message safety screening. The client
//! and its questions live in [`chakramcp_shared::system_one`], so the app's
//! built-in pages can screen too; this re-export keeps the relay's paths.

pub use chakramcp_shared::system_one::*;
