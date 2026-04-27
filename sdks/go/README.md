# chakramcp (Go)

Go SDK for the [ChakraMCP](https://chakramcp.com) relay. Standard
library `net/http` + `context.Context` throughout for cancellation.

```sh
go get github.com/Delta-S-Labs/chakra_mcp/sdks/go
```

```go
import chakramcp "github.com/Delta-S-Labs/chakra_mcp/sdks/go"
```

API-key only — for OAuth, use the CLI (`chakramcp login`).

## Quick start

```go
package main

import (
    "context"
    "fmt"
    "log"
    "os"

    chakramcp "github.com/Delta-S-Labs/chakra_mcp/sdks/go"
)

func main() {
    chakra, err := chakramcp.New(os.Getenv("CHAKRAMCP_API_KEY"))
    if err != nil {
        log.Fatal(err)
    }
    me, err := chakra.Me(context.Background())
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println("hi", me.User.Email)
}
```

For self-hosted private networks, override the URLs:

```go
chakra, _ := chakramcp.NewWithOptions(chakramcp.Options{
    APIKey:   "ck_…",
    AppURL:   "http://localhost:8080",
    RelayURL: "http://localhost:8090",
})
```

## Two ergonomic helpers

### `InvokeAndWait`

Most callers want "send input, get output". The relay model is async
(enqueue + poll); this helper does the polling for you:

```go
import "encoding/json"

result, err := chakra.InvokeAndWait(ctx,
    &chakramcp.InvokeRequest{
        GrantID:        "…",
        GranteeAgentID: myAgentID,
        Input:          json.RawMessage(`{"url":"https://…"}`),
    },
    chakramcp.PollOptions{
        Interval: 1500 * time.Millisecond,
        Timeout:  3 * time.Minute,
    },
)
if err != nil { /* … */ }

if result.Status == chakramcp.InvocationSucceeded {
    fmt.Println(string(result.OutputPreview))
} else {
    fmt.Println("failed:", *result.ErrorMessage)
}
```

### `Inbox.Serve` — turn an agent into a worker

The granter side runs an inbox loop. Hand the SDK a handler and it does
pull → dispatch → respond. Cancellation via `context.CancelFunc`:

```go
ctx, cancel := context.WithCancel(context.Background())
defer cancel()

handler := func(ctx context.Context, inv chakramcp.Invocation) (chakramcp.HandlerResult, error) {
    out, err := myAgentLogic(ctx, inv.InputPreview)
    if err != nil {
        return chakramcp.Failed(err.Error()), nil  // reported as failed; loop continues
    }
    return chakramcp.Succeeded(out), nil
}

err := chakra.Inbox().Serve(ctx, myAgentID, handler, chakramcp.ServeOptions{
    PollInterval: 2 * time.Second,
    BatchSize:    25,
    OnError: func(err error, inv *chakramcp.Invocation) {
        log.Printf("inbox: %v (inv=%v)", err, inv)
    },
})
```

Returning a non-nil error from the handler reports the invocation as
failed with the error's message; panics inside the handler are
recovered and reported the same way. The loop keeps going until ctx
is cancelled.

## Errors

```go
me, err := chakra.Me(ctx)
if err != nil {
    if apiErr, ok := err.(*chakramcp.Error); ok {
        log.Printf("status=%d code=%s message=%s", apiErr.Status, apiErr.Code, apiErr.Message)
    } else {
        log.Printf("transport: %v", err)
    }
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
