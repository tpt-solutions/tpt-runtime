# tpt-runtime-core

[![crate](https://img.shields.io/badge/crate-tpt--runtime--core-orange)](https://crates.io/crates/tpt-runtime-core)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--core-blue)](https://docs.rs/tpt-runtime-core)

Platform-independent runtime primitives shared by every `tpt-runtime` crate:
identifiers, the workload lifecycle state machine, errors, events, timestamps
and resource-usage accounting.

This is the bottom of the dependency graph. It has **no** concept of Windows,
WASM, OCI or any other backend — that constraint is what keeps the workload
model backend-independent (SPEC §8).

## Why a separate core crate?

Every other crate needs the same five things: a way to name a workload, a state
machine that says what may follow what, one error type that carries
attribution, one event shape, and one usage record. Putting them in a
dependency-free crate means:

- backends (`tpt-runtime-windows`, `tpt-runtime-wasm`, ...) never depend on
  each other, only on shared vocabulary;
- the API, daemon and CLI can be reasoned about without loading `wasmtime` or
  `windows-sys`;
- serialization conventions (snake_case, compact JSON) are defined once.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-core = "0.1"
```

## Identifiers

Every id is an opaque newtype over a string, so backends can use natural key
formats without leaking into the model. Generation is prefix-tagged UUIDv4.

```rust
use tpt_runtime_core::{DeviceId, WorkloadId, VolumeId};

let id = WorkloadId::generate();
assert!(id.as_str().starts_with("wl-"));       // wl-<uuid v4>

// Natural keys are accepted too, which keeps logs readable.
let project = VolumeId::from_raw("project");
assert_eq!(project.to_string(), "project");

let gpu = DeviceId::from_raw("gpu:0");
assert_eq!(gpu.as_str(), "gpu:0");

// All ids are transparent over JSON, so the wire format stays a plain string.
let json = serde_json::to_string(&id).unwrap();
assert!(json.starts_with("\"wl-"));
```

| Type | Prefix | Example |
| --- | --- | --- |
| `WorkloadId` | `wl` | `wl-6f96f5e2-...` |
| `VolumeId` | `vol` | `vol-project` |
| `NetworkId` | `net` | `net-0d1f...` |
| `DeviceId` | `dev` | `dev-gpu-0` |
| `CapabilityId` | `cap` | `cap-9a3c...` |
| `ServiceId` | `svc` | `svc-database` |
| `ResourceId` | `res` | `res-44b1...` |

Each type also exposes a `PREFIX` constant and `AsRef<str>`.

## Lifecycle state machine

`WorkloadState::transition` is the single authority on what may follow what. It
returns `Some(target)` when the move is permitted and `None` when it is not,
so callers cannot accidentally invent a transition.

```rust
use tpt_runtime_core::WorkloadState as S;

assert_eq!(S::Defined.transition(S::Running), None);           // not permitted
assert_eq!(S::Stopped.transition(S::Starting), Some(S::Starting)); // restart
assert_eq!(S::Destroyed.transition(S::Starting), None);         // terminal

// Failure is reachable from every active state.
for state in [S::Created, S::Starting, S::Running, S::Paused, S::Stopping] {
    assert_eq!(state.transition(S::Failed), Some(S::Failed));
}

assert!(S::Running.is_active());
assert!(S::Stopped.is_terminal());
```


The canonical progression is:

```text
defined → resolved → prepared → created → starting → running
        → paused ⇄ running
        → stopping → stopped → starting (restart)
        → destroyed (terminal)
```

`LIFECYCLE_TRANSITIONS` exposes the full table as a
`&[(WorkloadState, WorkloadState)]` if you need to inspect or render it.

## Errors

`RuntimeError` pairs a machine-readable `ErrorKind` with optional attribution,
so every significant failure can answer: which workload, which backend, which
operation.

```rust
use tpt_runtime_core::RuntimeError;
use tpt_runtime_core::error::ErrorKind;

let err = RuntimeError::new(ErrorKind::CapabilityDenied, "network.outbound not granted")
    .with_workload("wl-dev-agent")
    .with_backend("windows")
    .with_operation("start");

assert_eq!(err.kind, ErrorKind::CapabilityDenied);
assert_eq!(
    err.to_string(),
    "capability_denied [wl-dev-agent] (windows) during start: network.outbound not granted"
);

// Attributions are omitted from JSON when absent, keeping responses small.
let value: serde_json::Value = serde_json::to_value(&err).unwrap();
assert_eq!(value["kind"], "capability_denied");
```

`ErrorKind` covers: `InvalidConfiguration`, `InvalidTransition`, `NotFound`,
`CapabilityDenied`, `ResourceExhausted`, `BackendUnavailable`, `NotImplemented`,
`StorageFailure`, `NetworkFailure`, `DeviceUnavailable`, `System`, `Other`.
It implements `FromStr`, so wire errors map back to typed kinds.

`From<std::io::Error>` and `From<serde_json::Error>` are provided so `?` works
directly in fallible runtime code.

## Events

`RuntimeEvent` is the structured telemetry record (SPEC §28). Event names are
stable dotted strings, and payload fields are flattened into the top-level
object so logs stay readable.

```rust
use tpt_runtime_core::{EventKind, RuntimeEvent};

let event = RuntimeEvent::now(EventKind::WorkloadStopped)
    .with_workload("wl-dev-agent")
    .with_backend("windows")
    .with_field("exit_code", 0)
    .with_field("reason", "requested");

let value: serde_json::Value = serde_json::to_value(&event).unwrap();
assert_eq!(value["event"], "workload.stopped");
assert_eq!(value["exit_code"], 0);
assert_eq!(value["reason"], "requested");
assert!(value.get("fields").is_none());   // flattened, not nested
```

Round-tripping names works through `EventKind::from_str` / `as_str`:

```rust
use tpt_runtime_core::EventKind;

assert_eq!(EventKind::from_str("capability.denied"), Some(EventKind::CapabilityDenied));
assert_eq!(EventKind::CapabilityDenied.as_str(), "capability.denied");
assert_eq!(EventKind::from_str("nope"), None);
```

## Timestamps

```rust
use tpt_runtime_core::Timestamp;

let now = Timestamp::now();
assert!(now.unix_seconds() > 1_700_000_000);
```

## Resource usage

`ResourceUsage` records what a workload *actually* consumed, as distinct from
what it requested. Durations serialize as integer nanoseconds and every field is
skipped when zero, so idle workloads produce `{}`.

```rust
use std::time::Duration;
use tpt_runtime_core::ResourceUsage;

let usage = ResourceUsage {
    user_cpu: Duration::from_millis(1500),
    memory_peak_bytes: 4096,
    ..Default::default()
};

assert_eq!(usage.total_cpu(), Duration::from_millis(1500));

let value: serde_json::Value = serde_json::to_value(&usage).unwrap();
assert_eq!(value["user_cpu"], 1_500_000_000u64);

// Peak-style merging, used when combining samples.
let a = ResourceUsage { memory_peak_bytes: 100, ..Default::default() };
let b = ResourceUsage { memory_peak_bytes: 50, ..Default::default() };
assert_eq!(a.max_with(&b).memory_peak_bytes, 100);
```

## Testing

```console
cargo test -p tpt-runtime-core
```

Tests cover id prefixes and JSON round-trips, legal and illegal lifecycle
transitions, error attribution, event names, and usage peak merging.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.

