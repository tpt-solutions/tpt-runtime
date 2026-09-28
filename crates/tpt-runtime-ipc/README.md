# tpt-runtime-ipc

[![crate](https://img.shields.io/badge/crate-tpt--runtime--ipc-orange)](https://crates.io/crates/tpt-runtime-ipc)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--ipc-blue)](https://docs.rs/tpt-runtime-ipc)

IPC primitives shared by the daemon, API and CLI (SPEC §22, §30): the JSON
envelope for the local API, and newline-delimited JSON framing over any async
byte stream.

The framing is deliberately simple so the CLI, manual probing, a VS Code
extension (SPEC §38) and future transports can all share one protocol.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-ipc = "0.1"
```

## The envelope

Requests and responses correlate by `id`:

```rust
use tpt_runtime_ipc::Request;
use serde_json::Value;

let request = Request {
    id: 7,
    method: "workloads.list".to_owned(),
    params: Value::Null,
};

let json = serde_json::to_value(&request).unwrap();
assert_eq!(json["id"], 7);
assert_eq!(json["method"], "workloads.list");

// The envelope round-trips unchanged.
let back: Request = serde_json::from_value(json).unwrap();
assert_eq!(back, request);
```

## Responses carry exactly one of result or error

```rust
use serde_json::Value;
use tpt_runtime_ipc::Response;

let ok = Response::ok(1, Value::Bool(true));
let value = serde_json::to_value(&ok).unwrap();
assert_eq!(value["result"], true);
assert!(value.get("error").is_none());

let failed = Response::error(2, "not_found", "no such workload");
let value = serde_json::to_value(&failed).unwrap();
assert_eq!(value["error"]["kind"], "not_found");
assert!(value.get("result").is_none());
```

`Response::err` converts a `RuntimeError` directly, carrying its workload,
backend and operation attribution into the wire payload:

```rust
use serde_json::json;
use tpt_runtime_core::error::{ErrorKind, RuntimeError};
use tpt_runtime_ipc::Response;

let error = RuntimeError::new(ErrorKind::CapabilityDenied, "network.outbound not granted")
    .with_workload("wl-dev-agent")
    .with_backend("windows")
    .with_operation("start");

let value = serde_json::to_value(Response::err(3, &error)).unwrap();
assert_eq!(value["error"]["kind"], "capability_denied");
assert_eq!(value["error"]["workload"], "wl-dev-agent");
assert_eq!(value["error"]["backend"], "windows");
assert_eq!(value["error"]["operation"], "start");
```

## Framing

`write_message` writes one JSON document followed by a newline;
`read_message` reads one line and deserializes it, returning `None` at a clean
end of stream.

```rust,no_run
use tpt_runtime_ipc::{read_message, write_message, Request};
use serde_json::Value;

# async fn demo() -> tpt_runtime_core::Result<()> {
let (client, server) = tokio::io::duplex(4096);
let mut writer = client;
let mut reader = tokio::io::BufReader::new(server);

let request = Request {
    id: 1,
    method: "daemon.status".to_owned(),
    params: Value::Null,
};

write_message(&mut writer, &request).await?;

let echoed: Request = read_message(&mut reader).await?.unwrap();
assert_eq!(echoed.method, "daemon.status");
# Ok(())
# }
```

## The wire protocol

Transport is a Windows named pipe (`\\.\pipe\tpt-runtime-api`, overridable with
`TPT_RUNTIME_PIPE`). Messages are newline-delimited UTF-8 JSON:

```text
--> {"id": 1, "method": "workloads.list", "params": {}}
<-- {"id": 1, "result": [...]}
<-- {"event": "workload.started", "workload": "wl-...", "timestamp": "..."}
```

Events carry no `id` and flow server to client only, after `events.subscribe`.

## Methods

| Group | Methods |
| --- | --- |
| `daemon` | `status`, `shutdown` |
| `workloads` | `create`, `start`, `stop`, `restart`, `pause`, `destroy`, `list`, `inspect`, `logs`, `usage` |
| `events` | `history`, `subscribe` |
| `volumes` | `create`, `list`, `remove` |
| `devices` | `list` |
| `secrets` | `set`, `list`, `delete` |

## Swapping the transport

`read_message` and `write_message` are generic over any `AsyncBufRead` /
`AsyncWrite`, so an Archon shared-memory channel can replace the named-pipe
byte stream without touching API semantics.

## Testing

```console
cargo test -p tpt-runtime-ipc
```

Covers envelope shape, the result-or-error invariant, error attribution
mapping, and newline framing including a clean end of stream.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
