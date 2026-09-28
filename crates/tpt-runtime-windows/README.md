# tpt-runtime-windows

[![crate](https://img.shields.io/badge/crate-tpt--runtime--windows-orange)](https://crates.io/crates/tpt-runtime-windows)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--windows-blue)](https://docs.rs/tpt-runtime-windows)

The native Windows process backend (SPEC §12): first-class Windows workloads
with process-group semantics via Job Objects, environment management,
stdout/stderr capture and resource accounting.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-windows = "0.1"
```

Windows is the primary host. On other platforms the crate compiles but every
operation reports `BackendUnavailable`.

## Isolation properties

- **Job Objects.** Every workload process runs inside a dedicated Job Object,
  so termination is job-wide and child processes cannot outlive the workload.
- **Kill on close.** `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ties workload lifetime
  to the daemon's lifetime: a crashed daemon cannot leak workload processes.
- **Memory limits.** Enforced by the job when the manifest requests one.
- **No ambient authority.** The child environment is a sanitized allowlist plus
  the manifest's own values — it does not inherit the daemon's environment.

## Using the backend

```rust
use std::sync::Arc;
use tpt_runtime_process::ExecutionBackend;
use tpt_runtime_windows::WindowsProcessBackend;

let backend = WindowsProcessBackend::new();
assert_eq!(backend.kind(), tpt_runtime_model::BackendKind::Windows);
```

In the daemon, backends are registered with the workload manager:

```rust
use std::sync::Arc;
use tpt_runtime_workload::WorkloadManager;

fn register(manager: &WorkloadManager) {
    manager.register_backend(Arc::new(tpt_runtime_windows::WindowsProcessBackend::new()));
}
```

## The lifecycle end to end

```rust,no_run
use tpt_runtime_capability::CapabilitySet;
use tpt_runtime_core::WorkloadId;
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::network::NetworkMode;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, StartContext};
use tpt_runtime_windows::WindowsProcessBackend;

# fn run() -> tpt_runtime_core::Result<()> {
let backend = WindowsProcessBackend::new();

let spec = WorkloadSpec::new(
    "windows-echo",
    ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "cmd.exe".to_owned(),
        args: vec!["/C".to_owned(), "echo hello from windows".to_owned()],
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

backend.prepare(&spec, &ctx)?;          // idempotent, starts nothing
let instance = backend.start(&spec, &ctx)?;

// `exit` is async, so drive it from a runtime.
let status = tokio::runtime::Runtime::new()?.block_on(instance.exit()).map_err(|e| tpt_runtime_core::RuntimeError::new(tpt_runtime_core::error::ErrorKind::System, e.to_string()))?;
assert!(!status.failed);
# Ok(())
# }
```

## Resource accounting

The job object samples the counters that feed `ResourceUsage`. The job type
itself is an internal implementation detail of this crate, so it is not part of
the public API; the accounting it produces is.

A running instance reports the job's user and kernel CPU time, peak commit
charge, and bytes read and written:

```rust,no_run
use tpt_runtime_core::ResourceUsage;

# fn sample(instance: &dyn tpt_runtime_process::WorkloadInstance) -> tpt_runtime_core::Result<()> {
let usage: ResourceUsage = instance.stats()?;
println!("total cpu: {:?}", usage.total_cpu());
println!("peak memory: {} bytes", usage.memory_peak_bytes);
println!("read {} / written {}", usage.read_bytes, usage.write_bytes);
# Ok(())
# }
```

## Graceful stop

Native Windows processes have no portable graceful-shutdown contract, so this
backend treats both `StopMode::Graceful` and `StopMode::Kill` as job-wide
termination. This is documented rather than silently different from the trait's
wording.

## Non-Windows hosts

The crate compiles everywhere so the workspace builds on any platform. On a
non-Windows host `prepare` and `start` return `BackendUnavailable` rather than
panicking, which lets the runtime report the situation precisely.

## Testing

```console
cargo test -p tpt-runtime-windows
```

Unit tests cover the non-Windows paths and job-object configuration. Full
Windows behaviour is covered end to end by
`tpt-runtime-workload`'s `manager_e2e` tests.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
