# tpt-runtime-wasm

[![crate](https://img.shields.io/badge/crate-tpt--runtime--wasm-orange)](https://crates.io/crates/tpt-runtime-wasm)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--wasm-blue)](https://docs.rs/tpt-runtime-wasm)

WASM execution (SPEC §14) on [wasmtime](https://wasmtime.dev) with WASI
preview 1.

- **Sandbox by default.** A module sees only what is granted: explicitly
  preopened volumes, a small environment, and no sockets.
- **Deterministic resource limits.** CPU is bounded by wasmtime *fuel*
  (`resources.fuel`), wall-clock time by *epoch* interruption
  (`resources.timeout_secs`), and stop takes effect via an epoch trap.
- **Uniform capture.** WASI stdout/stderr flow into the same log buffers and
  files as every other backend, so `tpt logs` works unchanged.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-wasm = "0.1"
```

## Runtime classes

The manifest's `class` selects intent (SPEC §14):

| Class | Intent |
| --- | --- |
| `command` | run `_start` to completion |
| `service` | long-running; currently runs `_start` with epoch-based stop |
| `function` | invoked; currently runs `_start` as well |

`service` and `function` map onto long-running or invoked exports in a later
phase. For now they run `_start` too, stopped by epoch.

## Creating a backend

`WasmBackend::new` builds the engine with fuel metering and epoch interruption
enabled, and starts the background epoch ticker.

```rust
use tpt_runtime_process::ExecutionBackend;
use tpt_runtime_wasm::WasmBackend;

let backend = WasmBackend::new().unwrap();
assert_eq!(backend.kind(), tpt_runtime_model::BackendKind::Wasm);
```

The epoch ticker increments the engine epoch every 10 ms, so a stopped or
timed-out module traps within roughly 10 ms even in the middle of a tight
compute loop.

## Running a module

```rust,no_run
use tpt_runtime_capability::CapabilitySet;
use tpt_runtime_core::WorkloadId;
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::{WasmClass, WasmModuleSpec};
use tpt_runtime_model::network::NetworkMode;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, StartContext};
use tpt_runtime_wasm::WasmBackend;

# fn run() -> tpt_runtime_core::Result<()> {
let backend = WasmBackend::new()?;

let spec = WorkloadSpec::new(
    "wasm-hello",
    ExecutionSpec::WasmModule(WasmModuleSpec {
        module: std::path::PathBuf::from("examples/hello.wasm"),
        class: WasmClass::Command,
        ..Default::default()
    }),
);

let ctx = StartContext {
    workload_id: WorkloadId::generate(),
    mounts: Vec::new(),
    log_dir: std::env::temp_dir(),
    capabilities: CapabilitySet::from_names(["filesystem.read"]),
    network_mode: NetworkMode::None,
    exposed_ports: Vec::new(),
};

backend.prepare(&spec, &ctx)?;
let instance = backend.start(&spec, &ctx)?;
# Ok(())
# }
```

## Fuel and epoch limits

Fuel bounds deterministic work, so it is the limit to use when you want
reproducible behaviour. Epoch bounds wall-clock time.

```toml
[execution]
backend = "wasm"
module = "examples/hello.wasm"
class = "command"

[resources]
fuel = 100000        # deterministic instruction budget
timeout_secs = 30    # wall-clock ceiling, enforced by epoch interruption
```

`fuel` is optional: without it the module runs until `timeout_secs` fires.

## Preopened volumes

Read-write and read-only mounts become WASI preopened directories, which is how
a module gets filesystem access at all. A module with no mounts has no ambient
filesystem access beyond what WASI itself provides.

```rust,no_run
use std::path::PathBuf;

use tpt_runtime_model::volume::VolumeAccessMode;
use tpt_runtime_process::ResolvedMount;

let mount = ResolvedMount {
    name: "source".to_owned(),
    mount: "/workspace".to_owned(),
    host_path: PathBuf::from("/var/lib/tpt/volumes/source"),
    mode: VolumeAccessMode::ReadWrite,
};

assert_eq!(mount.name, "source");
assert_eq!(mount.mode, VolumeAccessMode::ReadWrite);
```

## Module formats

The backend accepts WAT text as well as binary `.wasm`. wasmtime parses both,
which keeps examples readable — the shipped `examples/hello.wasm` is in fact
WAT text.

## Isolation boundary

The WASM sandbox builder (preopens, limits) is intended to migrate into a
shared `tpt-boxcar` WASM service layer, so plugins across TPT projects get
identical semantics.

## Testing

```console
cargo test -p tpt-runtime-wasm
```

Integration tests in `tests/wasi.rs` cover hello-world execution, fuel
exhaustion, stop behaviour and spec validation.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
