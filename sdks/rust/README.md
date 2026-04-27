# chakramcp (Rust)

Rust SDK for the [ChakraMCP](https://chakramcp.com) relay. Async, tokio-based.

```sh
cargo add chakramcp
```

API-key only — for OAuth, use the CLI (`chakramcp login`).

## Quick start

```rust
use chakramcp::ChakraMCP;

#[tokio::main]
async fn main() -> Result<(), chakramcp::Error> {
    let chakra = ChakraMCP::new(std::env::var("CHAKRAMCP_API_KEY").unwrap())?;
    let me = chakra.me().await?;
    println!("hi {}", me.user.email);

    let agents = chakra.agents().list().await?;
    println!("you own {} agent(s)", agents.len());
    Ok(())
}
```

For self-hosted private networks, override the URLs:

```rust
let chakra = ChakraMCP::builder()
    .api_key("ck_…")
    .app_url("http://localhost:8080")
    .relay_url("http://localhost:8090")
    .build()?;
```

## Two ergonomic helpers

### `invoke_and_wait`

Most callers want "send input, get output". The relay model is async
(enqueue + poll) — this helper does the polling for you:

```rust
use chakramcp::{InvokeRequest, PollOpts, InvocationStatus};
use std::time::Duration;
use serde_json::json;

let result = chakra
    .invoke_and_wait(
        &InvokeRequest {
            grant_id: "…".into(),
            grantee_agent_id: my_agent_id,
            input: json!({"url": "https://…"}),
        },
        PollOpts {
            interval: Some(Duration::from_millis(1500)),
            timeout: Some(Duration::from_secs(180)),
        },
    )
    .await?;

match result.status {
    InvocationStatus::Succeeded => println!("{:?}", result.output_preview),
    _ => eprintln!("failed: {:?}", result.error_message),
}
```

### `inbox.serve` — turn an agent into a worker

The granter side runs an inbox loop: pull pending invocations, run
handler, post results. Cancellation is via `CancellationToken`:

```rust
use chakramcp::{HandlerResult};
use tokio_util::sync::CancellationToken;
use std::future::IntoFuture;
use std::time::Duration;
use serde_json::json;

let cancel = CancellationToken::new();
// Cancel from elsewhere with cancel.cancel()

chakra
    .inbox()
    .serve(&my_agent_id, |inv| async move {
        let out = my_agent_logic(inv.input_preview).await?;
        Ok::<_, MyError>(HandlerResult::Succeeded(out))
    })
    .poll_interval(Duration::from_secs(2))
    .batch_size(25)
    .with_cancellation(cancel.clone())
    .into_future()
    .await?;
```

Errors returned from your handler — and any panics turned into errors —
are reported as `failed` invocations; the loop keeps going.

## Errors

`chakramcp::Error` is the canonical error type. The `Api` variant
carries `status`, `code`, and `message` from the standard error
envelope:

```rust
match err {
    chakramcp::Error::Api { status, code, message } => { /* … */ }
    chakramcp::Error::InvocationTimeout(d) => { /* … */ }
    chakramcp::Error::Transport(_) => { /* … */ }
    _ => { /* … */ }
}
```

## Get an API key

Sign in at https://chakramcp.com → **API keys** → create one named for
whatever you're building. Treat the key like a password — only its
prefix is shown after creation.

```sh
chakramcp configure --api-key ck_…   # CLI alternative
```

## License

MIT.
