# tpt-runtime-workload

[![crate](https://img.shields.io/badge/crate-tpt--runtime--workload-orange)](https://crates.io/crates/tpt-runtime-workload)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--workload-blue)](https://docs.rs/tpt-runtime-workload)

The workload manager (SPEC §9, §26): one coherent lifecycle over every
execution backend.

This is the only component that knows all the backends. Everything above it —
the API, the daemon, the CLI — speaks in workload terms only, which is what
keeps the workload model backend-independent (SPEC §5.3).

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-workload = "0.1"
```

## Assembling a manager

`WorkloadManager::new` takes the subsystems it coordinates; backends are
registered separately with `register_backend`.

```rust,no_run
use std::sync::{Arc, Mutex};

use tpt_runtime_observe::{EventHub, MetricsRegistry};
use tpt_runtime_policy::{HostCapacity, PolicyEngine};
use tpt_runtime_security::SecretStore;
use tpt_runtime_storage::StorageManager;
use tpt_runtime_network::NetworkManager;
use tpt_runtime_device::DeviceRegistry;
use tpt_runtime_workload::WorkloadManager;

# fn build() -> tpt_runtime_core::Result<()> {
let state_dir = std::env::temp_dir().join("tpt-runtime");

let manager = WorkloadManager::new(
    state_dir.join("logs"),
    Arc::new(EventHub::new(Some(state_dir.join("events.jsonl")))),
    Arc::new(MetricsRegistry::new()),
    Arc::new(Mutex::new(StorageManager::open(state_dir.join("volumes"))?)),
    Arc::new(NetworkManager::new()),
    Arc::new(Mutex::new(DeviceRegistry::new())),
    Arc::new(Mutex::new(SecretStore::open(state_dir.join("secrets.json"))?)),
    PolicyEngine::new(HostCapacity::unknown()),
);
# Ok(())
# }
```

## Registering backends

```rust,no_run
use std::sync::Arc;

use tpt_runtime_workload::WorkloadManager;

# fn register(manager: &WorkloadManager) -> tpt_runtime_core::Result<()> {
manager.register_backend(Arc::new(tpt_runtime_windows::WindowsProcessBackend::new()));
manager.register_backend(Arc::new(tpt_runtime_wasm::WasmBackend::new()?));
manager.register_backend(Arc::new(tpt_runtime_oci::OciBackend::new("/var/lib/tpt/oci")?));
manager.register_backend(Arc::new(tpt_runtime_linux::LinuxBackend::new()));
# Ok(())
# }
```

The manager dispatches on `BackendKind`, so adding a backend here is all that
is needed to make it runnable through the API and CLI.

## The lifecycle

```rust,no_run
use std::time::Duration;

use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_workload::{LogsQuery, WorkloadManager};

# fn lifecycle(manager: &WorkloadManager) -> tpt_runtime_core::Result<()> {
let spec = WorkloadSpec::new(
    "windows-echo",
    ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "cmd.exe".to_owned(),
        args: vec!["/C".to_owned(), "echo hello".to_owned()],
        ..Default::default()
    }),
);

// create -> started -> running, with validation and admission applied.
let id = manager.create(spec)?;
manager.start(&id)?;

// Inspect by id or by name.
let info = manager.inspect(id.as_str())?;
println!("{} is {}", info.name, info.state);

println!("stdout:");
for line in manager.logs(&id, LogsQuery::Stdout, 50)? {
    println!("  {line}");
}

// Wait for exit with a ceiling.
let status = manager.wait(&id, Duration::from_secs(30))?;
assert!(!status.failed);

// Tear the records down.
manager.destroy(&id)?;
# Ok(())
# }
```

## Inspecting workloads

```rust,no_run
use tpt_runtime_core::WorkloadId;
use tpt_runtime_workload::WorkloadManager;

# fn inspect(manager: &WorkloadManager, id: &WorkloadId) -> tpt_runtime_core::Result<()> {
// Summaries, for `tpt list`.
let all = manager.list();
for info in &all {
    println!("{} {} ({})", info.id, info.name, info.backend);
}

// Everything about one workload, for `tpt inspect`.
let info = manager.inspect(id.as_str())?;
println!("state:    {}", info.state);
println!("backend:  {}", info.backend);
println!("caps:     {:?}", info.capabilities);
println!("network:  {}", info.network_mode);
println!("ports:    {:?}", info.exposed_ports);
for mount in &info.mounts {
    println!("mount:    {} -> {} ({})", mount.name, mount.mount, mount.mode);
}

// Latest usage snapshot from the metrics registry.
let usage = manager.usage(id)?;
println!("peak memory: {} bytes", usage.memory_peak_bytes);
println!("total cpu:   {:?}", usage.total_cpu());
# Ok(())
# }
```

## Control operations

```rust,no_run
use std::time::Duration;

use tpt_runtime_core::WorkloadId;
use tpt_runtime_workload::WorkloadManager;

# fn control(manager: &WorkloadManager, id: &WorkloadId) -> tpt_runtime_core::Result<()> {
manager.restart(id)?;

// Stop and wait in one step, for a clean shutdown.
let status = manager.stop_and_wait(id, Duration::from_secs(10))?;
println!("exit: {:?}", status);

// pause/resume report not_implemented until a backend supports suspension.
# Ok(())
# }
```

## Accessing subsystems

The manager exposes the registries it coordinates, so the API layer can serve
volumes, devices and secrets without holding its own references.

```rust,no_run
use tpt_runtime_workload::WorkloadManager;

# fn subsystems(manager: &WorkloadManager) -> tpt_runtime_core::Result<()> {
let _events = manager.events();     // &EventHub
let _metrics = manager.metrics();   // &MetricsRegistry
let _storage = manager.storage();   // &Arc<Mutex<StorageManager>>
let _network = manager.network();   // &Arc<NetworkManager>
let _devices = manager.devices();   // &Arc<Mutex<DeviceRegistry>>
let _secrets = manager.secrets();   // &Arc<Mutex<SecretStore>>
# Ok(())
# }
```

## What the manager guarantees

- **The state machine is enforced.** Every transition goes through
  `tpt_runtime_core::WorkloadState::transition`, so an illegal move fails with
  `InvalidTransition` rather than corrupting a record.
- **Resources resolve before start.** Volumes, networks and devices are
  resolved and claimed before the backend is asked to start anything.
- **Capabilities are recorded.** Granted capability names appear in
  `WorkloadInfo`, so a workload's authority is inspectable.
- **Events and metrics are normalized.** Every backend's telemetry flows
  through `tpt-runtime-observe`, so observability does not vary by backend.
- **Failures are explicit.** A workload that cannot start fails with a reason,
  and the record moves to `failed` rather than disappearing.

## Testing

```console
cargo test -p tpt-runtime-workload
```

Unit tests cover state transitions and manager bookkeeping;
`tests/manager_e2e.rs` runs the manager against the real Windows backend,
covering create/start/logs/stop/destroy and the crash watcher that emits
`workload.failed`.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
