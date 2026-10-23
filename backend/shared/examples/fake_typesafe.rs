//! A local stand-in for TypeSafe's System One API, for trying message
//! safety without a key. Every question gets a low probability, except
//! the ones named in `FAKE_TYPESAFE_NOULS` (`contact_0=0.95,harm_crime=0.9`).
//!
//! ```text
//! cargo run -p chakramcp-shared --features test-support --example fake_typesafe
//! TYPESAFE_AI_BASE_URL=http://127.0.0.1:8099 TYPESAFE_AI_KEY=fake …
//! ```

use chakramcp_shared::system_one::test_support::{serve_on, Fake};

#[tokio::main]
async fn main() {
    let addr = std::env::var("FAKE_TYPESAFE_ADDR").unwrap_or_else(|_| "127.0.0.1:8099".into());
    let mut fake = Fake::default();
    for pair in std::env::var("FAKE_TYPESAFE_NOULS")
        .unwrap_or_default()
        .split(',')
    {
        if let Some((id, p)) = pair.split_once('=') {
            if let Ok(p) = p.trim().parse::<f64>() {
                // The fake keys answers by `&'static str`; this runs once.
                fake.nouls
                    .insert(Box::leak(id.trim().to_owned().into_boxed_str()), p);
            }
        }
    }
    let url = serve_on(fake, &addr).await;
    println!("fake TypeSafe on {url}");
    tokio::signal::ctrl_c().await.ok();
}
