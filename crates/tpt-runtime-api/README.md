# tpt-runtime-api

[![crate](https://img.shields.io/badge/crate-tpt--runtime--api-orange)](https://crates.io/crates/tpt-runtime-api)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--api-blue)](https://docs.rs/tpt-runtime-api)

The local API (SPEC §30): a named-pipe JSON server and the client the CLI
uses. Every `tpt` command is a client of this API — the CLI holds no runtime
state of its own.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-api = "0.1"
```

## Transport

| | |
| --- | --- |
| Default pipe | `\\.\pipe\tpt-runtime-api` |
| Override | `TPT_RUNTIME_PIPE`, or `DaemonConfig::pipe_name` |
| Framing | newline-delimited UTF-8 JSON |
| Concurrency | `DaemonConfig::max_clients`, default 32 |

## The client

`ApiClient::connect` retries for up to three seconds, because the daemon
creates pipe instances on demand. If it cannot connect, the error tells you how
to start the daemon.

```rust,no_run
use tpt_runtime_api::ApiClient;
use tpt_runtime_config::DaemonConfig;

# async fn demo() -> tpt_runtime_core::Result<()> {
let config = DaemonConfig::from_env();
let mut client = ApiClient::connect(&config).await?;

// Any method, any params.
let workloads = client.call("workloads.list", serde_json::Value::Null).await?;
println!("{workloads}");
# Ok(())
# }
```

A failed call comes back as a typed `RuntimeError`, so callers can branch on
`ErrorKind` rather than parsing strings.

```rust,no_run
use tpt_runtime_api::ApiClient;
use tpt_runtime_config::DaemonConfig;
use tpt_runtime_core::error::ErrorKind;

# async fn demo() -> tpt_runtime_core::Result<()> {
let mut client = ApiClient::connect(&DaemonConfig::from_env()).await?;

match client.call("workloads.inspect", serde_json::json!({ "id": "nope" })).await {
    Ok(value) => println!("{value}"),
    Err(err) if err.kind == ErrorKind::NotFound => println!("no such workload"),
    Err(err) => return Err(err),
}
# Ok(())
# }
```

## Subscribing to events

`subscribe` switches the connection to also receive server-pushed events.
Events have no correlation `id`, so a subscribed connection must read them
interleaved with responses.

```rust,no_run
use tpt_runtime_api::ApiClient;
use tpt_runtime_config::DaemonConfig;

# async fn demo() -> tpt_runtime_core::Result<()> {
let mut client = ApiClient::connect(&DaemonConfig::from_env()).await?;
client.subscribe().await?;

loop {
    let event = client.recv_event().await?;
    println!("{} {:?}", event.event, event.workload);
    if event.event == tpt_runtime_core::EventKind::WorkloadStopped {
        break;
    }
}
# Ok(())
# }
```

Unsubscribed calls are safe: `call` skips interleaved event lines for a
connection that did not subscribe, and matches responses by `id`.

## The server

`ApiState` bundles everything the server needs, and `serve` runs until a client
calls `daemon.shutdown` or the shutdown watch fires.

```rust,no_run
use std::sync::Arc;

use tpt_runtime_api::server::{serve, ApiState};
use tpt_runtime_config::DaemonConfig;
use tpt_runtime_workload::WorkloadManager;

# async fn run(manager: Arc<WorkloadManager>) -> tpt_runtime_core::Result<()> {
let (shutdown, _rx) = tokio::sync::watch::channel(false);

let state = Arc::new(ApiState {
    manager,
    started_at: std::time::Instant::now(),
    shutdown,
});

serve(DaemonConfig::from_env(), state).await
# }
```

The server is the first pipe instance, so a second daemon on the same pipe
fails immediately instead of silently splitting clients.

## Methods

| Group | Methods |
| --- | --- |
| `daemon` | `status`, `shutdown` |
| `workloads` | `create`, `start`, `stop`, `restart`, `pause`, `destroy`, `list`, `inspect`, `logs`, `usage` |
| `events` | `history`, `subscribe` |
| `volumes` | `create`, `list`, `remove` |
| `devices` | `list` |
| `secrets` | `set`, `list`, `delete` |

The full protocol — including the manual-probing wire format — is documented in
`docs/ARCHITECTURE.md` in the repository.

## Probing by hand

Because the framing is plain newline-delimited JSON, the API can be exercised
without the CLI, which is what makes it easy to debug:

```text
--> {"id": 1, "method": "daemon.status", "params": null}
<-- {"id": 1, "result": {...}}
```

## Non-Windows hosts

The client and server compile on non-Windows hosts, where the transport falls
back to a loopback TCP socket, so the workspace builds and the API can be
exercised during development.

## Testing

```console
cargo test -p tpt-runtime-api
```

Covers the envelope, response shaping, method dispatch and error mapping.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
