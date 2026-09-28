# tpt-runtime-network

[![crate](https://img.shields.io/badge/crate-tpt--runtime--network-orange)](https://crates.io/crates/tpt-runtime-network)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--network-blue)](https://docs.rs/tpt-runtime-network)

Logical network abstraction: turning the network *intent* in a manifest into a
concrete allocation of host ports (SPEC §18).

The model says what a workload wants (`outbound`, `service`, `host`). This
crate answers "which host ports, then", and refuses rather than guessing.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-network = "0.1"
tpt-runtime-model = "0.1"
tokio = { version = "1", features = ["rt", "net"] }
```

Examples below are wrapped in
`tokio::runtime::Runtime::new().unwrap().block_on(async { ... })` because
`assign` is async.

## Allocating ports

`assign` is async because dynamic ports are chosen by binding an ephemeral
socket and reading back what the OS gave.

```rust,no_run
use std::collections::BTreeMap;
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_network::NetworkManager;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let manager = NetworkManager::new();

let spec = NetworkSpec {
    mode: NetworkMode::Service,
    expose: BTreeMap::from([
        ("http".to_owned(), 8080u16),   // fixed port
        ("metrics".to_owned(), 0u16),   // 0 means "pick a free one"
    ]),
};

let assignment = manager.assign("api", &spec).await.unwrap();
assert_eq!(assignment.mode, NetworkMode::Service);
assert_eq!(assignment.ports.len(), 2);

for port in &assignment.ports {
    println!("{} -> {} (static: {})", port.name, port.host_port, port.static_port);
}
# });
```

A requested port of `0` is dynamic and reported with `static_port: false`;
a non-zero request is honoured exactly and reported as static.

## Modes that cannot accept inbound traffic fail explicitly

Exposing a port under `Outbound` or `None` is a configuration error, not
something to silently ignore (SPEC §48).

```rust,no_run
use std::collections::BTreeMap;
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_network::NetworkManager;
use tpt_runtime_core::error::ErrorKind;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let manager = NetworkManager::new();

let spec = NetworkSpec {
    mode: NetworkMode::Outbound,
    expose: BTreeMap::from([("http".to_owned(), 80u16)]),
};

let err = manager.assign("api", &spec).await.unwrap_err();
assert_eq!(err.kind, ErrorKind::NetworkFailure);
assert!(err.message.contains("cannot expose ports"));
# });
```

`None` mode succeeds and simply allocates nothing:

```rust,no_run
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_network::NetworkManager;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let manager = NetworkManager::new();
let assignment = manager
    .assign("sandboxed", &NetworkSpec { mode: NetworkMode::None, ..Default::default() })
    .await
    .unwrap();

assert!(assignment.ports.is_empty());
# });
```

## Releasing ports

`release` frees the ports an assignment holds, which is what makes a fixed port
available to the next workload after the first stops.

```rust,no_run
use std::collections::BTreeMap;
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_network::NetworkManager;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let manager = NetworkManager::new();
let spec = NetworkSpec {
    mode: NetworkMode::Service,
    expose: BTreeMap::from([("http".to_owned(), 8080u16)]),
};

let first = manager.assign("api", &spec).await.unwrap();
manager.release(&first);

// The same static port can now be claimed again.
let second = manager.assign("api2", &spec).await.unwrap();
assert_eq!(second.ports[0].host_port, 8080);
# });
```

## Duplicate static ports are rejected

Two workloads cannot hold the same fixed port. The second request fails with
`NetworkFailure` instead of stealing the port.

```rust,no_run
use std::collections::BTreeMap;
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_network::NetworkManager;
use tpt_runtime_core::error::ErrorKind;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let manager = NetworkManager::new();
let spec = NetworkSpec {
    mode: NetworkMode::Service,
    expose: BTreeMap::from([("http".to_owned(), 9000u16)]),
};

manager.assign("first", &spec).await.unwrap();
let err = manager.assign("second", &spec).await.unwrap_err();
assert_eq!(err.kind, ErrorKind::NetworkFailure);
# });
```

## Service meshes

`NetworkMode::Service` currently allocates host ports. A `tpt-boxcar` service
mesh would take over resolution while the manifest intent stays identical —
this crate is the seam that allocation moves behind.

## Testing

```console
cargo test -p tpt-runtime-network
```

Covers `None` mode, static and dynamic port allocation, duplicate static port
rejection, expose-without-inbound failure, and port release.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
