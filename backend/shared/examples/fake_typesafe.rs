//! A local stand-in for TypeSafe's System One API, for trying message
//! safety without a key: every question gets the same low probability.
//!
//! ```text
//! cargo run -p chakramcp-shared --features test-support --example fake_typesafe
//! TYPESAFE_AI_BASE_URL=http://127.0.0.1:8099 TYPESAFE_AI_KEY=fake …
//! ```

use chakramcp_shared::system_one::test_support::Fake;

#[tokio::main]
async fn main() {
    let addr = std::env::var("FAKE_TYPESAFE_ADDR").unwrap_or_else(|_| "127.0.0.1:8099".into());
    let url = chakramcp_shared::system_one::test_support::serve_on(Fake::default(), &addr).await;
    println!("fake TypeSafe on {url}");
    tokio::signal::ctrl_c().await.ok();
}
