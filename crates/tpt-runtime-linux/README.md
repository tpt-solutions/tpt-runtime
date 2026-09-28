# tpt-runtime-linux

[![crate](https://img.shields.io/badge/crate-tpt--runtime--linux-orange)](https://crates.io/crates/tpt-runtime-linux)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--linux-blue)](https://docs.rs/tpt-runtime-linux)

Linux execution environments (SPEC §11), delivered through a layered strategy.

- **Phase 1 (today).** Run Linux workloads inside *existing* Windows
  virtualization via WSL (`wsl.exe`), exposed through the common workload
  model.
- **Phase 2.** Tighter resource-management integration.
- **Phase 3.** A TPT-managed Linux execution environment.

The runtime deliberately distinguishes Linux *compatibility* from Linux
*implementation*: a workload only needs a Linux-compatible environment, not a
Linux kernel under TPT's control.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-linux = "0.1"
```

## Using the backend

```rust
use tpt_runtime_linux::LinuxBackend;
use tpt_runtime_process::ExecutionBackend;

let backend = LinuxBackend::new();
assert_eq!(backend.kind(), tpt_runtime_model::BackendKind::Linux);
```

## Declaring a Linux workload

```toml
api = "tpt.runtime/v1"

[workload]
name = "linux-builder"

[execution]
backend = "linux"
distro = "ubuntu"
command = "/bin/sh -c 'make -j4'"
```

## Requirements

WSL must be installed and a named distribution available (`wsl.exe -l -v`).
Without it, workloads on this backend fail explicitly with a message pointing
at the missing prerequisite rather than failing obscurely.

## Scope and honest limits

Isolation *inside* the distro is out of scope for this crate. WSL provides the
boundary; capability grants are recorded and audited by the runtime, but this
backend does not itself enforce them within the distribution. Work that needs
per-workload isolation or resource control inside Linux belongs in a later
phase, or in `tpt-boxcar`.

## Testing

```console
cargo test -p tpt-runtime-linux
```

Covers spec validation and the paths that do not require a live WSL
installation. End-to-end behaviour needs WSL on the host.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
