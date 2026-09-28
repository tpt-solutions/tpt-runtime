# tpt-runtime-observe

[![crate](https://img.shields.io/badge/crate-tpt--runtime-observe-orange)](https://crates.io/crates/tpt-runtime-observe)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime-observe-blue)](https://docs.rs/tpt-runtime-observe)

Observability built in, not bolted on (SPEC §27): structured `RuntimeEvent`s
broadcast to subscribers and persisted as JSONL, plus per-workload usage
snapshots.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-observe = "0.1"
tokio = { version = "1", features = ["sync", "rt", "macros"] }
```

## The event hub

`EventHub::new` takes an optional sink path. When given one, every event is
appended as JSONL, so history survives a daemon restart.

```rust
use tpt_runtime_core::{EventKind, RuntimeEvent};
use tpt_runtime_observe::EventHub;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let path = std::env::temp_dir()
    .join(format!("tpt-events-{}.jsonl", std::process::id()));

let hub = EventHub::new(Some(&path));

// Subscribe before emitting to observe the live stream.
let mut subscriber = hub.subscribe();

hub.emit(
    RuntimeEvent::now(EventKind::WorkloadStarted)
        .with_workload("wl-dev-agent")
        .with_backend("windows"),
);

let event = subscriber.recv().await.unwrap();
assert_eq!(event.event, EventKind::WorkloadStarted);
assert_eq!(event.workload.as_ref().unwrap().as_str(), "wl-dev-agent");
# });
```

Broadcast is the right shape here: every subscriber sees every event, and a
slow subscriber does not stall the runtime.

## Reading history

History is read back from the JSONL sink, so `tpt events` can show what
happened before the current daemon started.

```rust
use tpt_runtime_core::{EventKind, RuntimeEvent};
use tpt_runtime_observe::EventHub;

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let path = std::env::temp_dir()
    .join(format!("tpt-events-history-{}.jsonl", std::process::id()));

let hub = EventHub::new(Some(&path));
hub.emit(RuntimeEvent::now(EventKind::WorkloadCreated).with_workload("wl-a"));
hub.emit(RuntimeEvent::now(EventKind::WorkloadStarted).with_workload("wl-a"));
hub.emit(RuntimeEvent::now(EventKind::WorkloadStopped).with_workload("wl-a"));

let history = hub.read_history(10).unwrap();
assert_eq!(history.len(), 3);
assert_eq!(history[0].event, EventKind::WorkloadCreated);

// The limit returns the most recent events.
let recent = hub.read_history(1).unwrap();
assert_eq!(recent.len(), 1);
assert_eq!(recent[0].event, EventKind::WorkloadStopped);
# });
```

## The metrics registry

The registry keeps the **latest** snapshot per workload. Backends push; the
API serves; nothing polls aggressively, which keeps telemetry overhead minimal
(SPEC §47).

```rust
use std::time::Duration;

use tpt_runtime_core::{ResourceUsage, WorkloadId};
use tpt_runtime_observe::MetricsRegistry;

let registry = MetricsRegistry::new();
let id = WorkloadId::generate();

assert!(registry.get(&id).is_none());

registry.update(&id, ResourceUsage {
    user_cpu: Duration::from_millis(500),
    memory_peak_bytes: 4096,
    ..Default::default()
});

let usage = registry.get(&id).unwrap();
assert_eq!(usage.memory_peak_bytes, 4096);
assert_eq!(usage.total_cpu(), Duration::from_millis(500));

// A later snapshot replaces the earlier one.
registry.update(&id, ResourceUsage { memory_peak_bytes: 8192, ..Default::default() });
assert_eq!(registry.get(&id).unwrap().memory_peak_bytes, 8192);

// All snapshots, for `tpt list` and `tpt status`.
assert_eq!(registry.all().len(), 1);

// Forgetting a destroyed workload's metrics.
registry.remove(&id);
assert!(registry.get(&id).is_none());
```

## Sampling from a running workload

`collect_from` takes a sampler closure and silently skips failures, so a
transient sampling error never breaks the metrics path.

```rust
use tpt_runtime_core::{ResourceUsage, RuntimeError, WorkloadId, error::ErrorKind};
use tpt_runtime_observe::MetricsRegistry;

let registry = MetricsRegistry::new();
let id = WorkloadId::generate();

// A failing sampler is ignored rather than propagated.
registry.collect_from(&id, || {
    Err(RuntimeError::new(ErrorKind::System, "job query failed"))
});
assert!(registry.get(&id).is_none());

// A successful sample is recorded.
registry.collect_from(&id, || Ok(ResourceUsage { memory_peak_bytes: 1024, ..Default::default() }));
assert_eq!(registry.get(&id).unwrap().memory_peak_bytes, 1024);
```

## From the CLI

```console
tpt events --limit 30     # recent history
tpt events --follow       # live stream until interrupted
```

## Accounting feeds

Job-object sampling on Windows and engine counters in the WASM backend both
push the same `ResourceUsage` records here. An Archon accounting feed would do
the same, without any change above this layer.

## Testing

```console
cargo test -p tpt-runtime-observe
```

Covers event broadcast, JSONL persistence and history reads, metrics
update/get/remove, and `collect_from` error tolerance.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
