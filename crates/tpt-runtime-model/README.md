# tpt-runtime-model

[![crate](https://img.shields.io/badge/crate-tpt--runtime--model-orange)](https://crates.io/crates/tpt-runtime-model)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--model-blue)](https://docs.rs/tpt-runtime-model)

The backend-independent workload model: `WorkloadSpec`, execution payloads,
resource requests, volumes, networks and devices (SPEC §9, §10, §18, §19, §25).

A `WorkloadSpec` says **what** a workload requires and **what it is allowed to
access**. It never says how a backend should realize those requirements — that
is what keeps Windows, Linux, OCI and WASM workloads describable by one type.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-model = "0.1"
```

## The central idea: requirements, not mechanisms

`ExecutionSpec` is an enum with exactly one populated variant, selected by the
manifest's `backend` key. The variants carry requirements only:

```rust
use tpt_runtime_model::{BackendKind, ExecutionSpec, execution::WindowsProcessSpec};

let execution = ExecutionSpec::WindowsProcess(WindowsProcessSpec {
    program: "myapp.exe".to_owned(),
    args: vec!["--serve".to_owned()],
    ..Default::default()
});

// The spec knows which backend runs it, but says nothing about how.
assert_eq!(execution.backend(), BackendKind::Windows);
```

`BackendKind` values are `Windows`, `Linux`, `Oci`, `Wasm`, and stringify to
the same names the manifest uses.

## Building a workload

`WorkloadSpec::new` starts from sensible defaults; fill in the rest afterwards.
`validate` checks the whole specification and returns the first problem it
finds.

```rust
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_model::resources::Memory;
use tpt_runtime_model::volume::{VolumeAccessMode, VolumeMount};
use std::collections::BTreeMap;

let mut spec = WorkloadSpec::new(
    "build-agent",
    ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "myapp.exe".to_owned(),
        ..Default::default()
    }),
);

spec.resources = tpt_runtime_model::resources::ResourceSpec {
    cpu: Some(4.0),
    memory: Some(Memory::gib(4)),
    ..Default::default()
};
spec.network = NetworkSpec {
    mode: NetworkMode::Outbound,
    ..Default::default()
};
spec.volumes = vec![VolumeMount {
    name: "project".to_owned(),
    mount: "/workspace".to_owned(),
    mode: VolumeAccessMode::ReadWrite,
}];
spec.capabilities = vec!["network.outbound".to_owned()];
spec.labels = BTreeMap::from([("team".to_owned(), "platform".to_owned())]);

assert!(spec.validate().is_ok());
```

Validation rejects empty or over-long names, names outside `[A-Za-z0-9._-]`,
volume mounts missing a name or mount point, and — importantly — exposed ports
on a network mode that cannot accept inbound traffic:

```rust
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use std::collections::BTreeMap;

// Outbound mode cannot accept inbound traffic, so exposing a port is a
// configuration error rather than something to silently ignore.
let spec = WorkloadSpec {
    name: "api".to_owned(),
    execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "api.exe".to_owned(),
        ..Default::default()
    }),
    resources: Default::default(),
    network: NetworkSpec {
        mode: NetworkMode::Outbound,
        expose: BTreeMap::from([("http".to_owned(), 8080u16)]),
    },
    volumes: Vec::new(),
    devices: Vec::new(),
    capabilities: Vec::new(),
    labels: BTreeMap::new(),
};

let err = spec.validate().unwrap_err();
assert_eq!(err.kind, ErrorKind::InvalidConfiguration);
assert!(err.message.contains("cannot expose ports"));
```

## Human-readable memory sizes

`Memory` is a byte count that also parses `512MiB`, `4GiB`, `1KiB` and bare
bytes. It always serializes as plain bytes, so the wire format stays canonical.

```rust
use tpt_runtime_model::resources::Memory;
use std::str::FromStr;

assert_eq!(Memory::from_str("4GiB").unwrap(), Memory::gib(4));
assert_eq!(Memory::from_str("512MiB").unwrap(), Memory::mib(512));
assert_eq!(Memory::from_str("1024").unwrap(), Memory(1024));
assert_eq!(Memory::gib(4).to_string(), "4GiB");

// Errors are explicit, never silently zero.
assert!(Memory::from_str("4 furlongs").is_err());
assert!(Memory::from_str("banana").is_err());
```

## Network modes

`NetworkMode` defaults to `None` — deny by default (SPEC §5.2). The
`allows_outbound` / `allows_inbound` predicates let callers reason about a mode
without pattern matching, and are what the validator and the network manager
both rely on.

| Mode | Outbound | Inbound |
| --- | --- | --- |
| `None` | no | no |
| `Outbound` | yes | no |
| `Service` | yes | yes |
| `Host` | yes | yes |

## Volumes

`VolumeMount` binds a logical volume name to a mount point with an access mode.
The model never resolves the name to a host path — that happens later, per
workload instance, via `tpt_runtime_process::ResolvedMount` (SPEC §15: one
volume, many views).

```rust
use tpt_runtime_model::volume::{VolumeAccessMode, VolumeMount};

let read_only = VolumeMount {
    name: "assets".to_owned(),
    mount: "/opt/assets".to_owned(),
    mode: VolumeAccessMode::ReadOnly,
};
assert_eq!(read_only.name, "assets");
```

## Devices

`DeviceRequest` pairs a logical device id such as `gpu:0` with a
`DeviceAccessMode` (`Compute`, `Full`, `ReadOnly`).

## OCI image references

`ImageReference` parses `name[:tag][@digest]`, including registry hosts. It
deliberately treats a trailing `:tag` as a tag only when it contains no `/`, so
ports in `localhost:5000/app` stay part of the repository.

```rust
use tpt_runtime_model::execution::oci_ref::ImageReference;

let simple = ImageReference::parse("postgres:16").unwrap();
assert_eq!(simple.repository, "postgres");
assert_eq!(simple.tag.as_deref(), Some("16"));
assert_eq!(simple.to_string(), "postgres:16");

// A registry port is not a tag.
let local = ImageReference::parse("localhost:5000/app").unwrap();
assert_eq!(local.repository, "localhost:5000/app");
assert_eq!(local.tag, None);

// Digests must be sha256.
assert!(ImageReference::parse("app@md5:deadbeef").is_err());
```

## Serialization

The whole model is `serde`-round-trippable, which is what lets a manifest, an
API request and a stored record all share one representation.

```rust
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;

let spec = ExecutionSpec::WindowsProcess(WindowsProcessSpec {
    program: "myapp.exe".to_owned(),
    args: vec!["--serve".to_owned()],
    ..Default::default()
});

let json = serde_json::to_string(&spec).unwrap();
let back: ExecutionSpec = serde_json::from_str(&json).unwrap();
assert_eq!(back, spec);
```

## Testing

```console
cargo test -p tpt-runtime-model
```

Tests cover spec validation (empty/illegal names, ports on outbound-only
modes), memory size parsing, image reference parsing including registry ports
and digest validation, and JSON round-trips.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
