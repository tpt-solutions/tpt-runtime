# tpt-runtime-process

[![crate](https://img.shields.io/badge/crate-tpt--runtime--process-orange)](https://crates.io/crates/tpt-runtime-process)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--process-blue)](https://docs.rs/tpt-runtime-process)

The execution backend interface (SPEC §10) plus the pieces every backend needs:
the prepare/start contexts, the `WorkloadInstance` handle a backend returns,
exit statuses, and log capture buffers.

If you are adding a new execution backend, this is the crate to depend on.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-process = "0.1"
```

## The two traits

```text
trait ExecutionBackend {
    fn kind(&self) -> BackendKind;
    fn prepare(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<()>;
    fn start(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>>;
}

trait WorkloadInstance {
    fn stats(&self) -> Result<ResourceUsage>;
    fn stop(&self, mode: StopMode) -> Result<()>;
    fn exit(&self) -> oneshot::Receiver<ExitStatus>;
    fn describe(&self) -> serde_json::Value { /* default: null */ }
}
```

`prepare` must be **idempotent** and must not start anything; it covers
`defined → resolved → prepared`. `start` returns a live handle. The methods are
synchronous by design: a backend does its own threading and the manager invokes
these off the async runtime's core threads.

## Implementing a backend

A complete, working backend. This one runs a command through the operating
system shell, which keeps the example short while exercising every part of the
contract.

```rust
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use tpt_runtime_core::error::Result;
use tpt_runtime_core::{ResourceUsage, RuntimeError};
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::{BackendKind, ExecutionSpec};
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, ExitStatus, StartContext, StopMode, WorkloadInstance};
use tokio::sync::oneshot;

/// A minimal backend: one process per workload.
struct DemoBackend {
    pid: Mutex<Option<u32>>,
}

impl DemoBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self { pid: Mutex::new(None) })
    }
}

impl ExecutionBackend for DemoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Windows
    }

    fn prepare(&self, _spec: &WorkloadSpec, _ctx: &StartContext) -> Result<()> {
        // Validate that the executable exists, create scratch space, etc.
        // Must be idempotent and must not start the workload.
        Ok(())
    }

    fn start(&self, spec: &WorkloadSpec, _ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>> {
        let ExecutionSpec::WindowsProcess(WindowsProcessSpec { program, args, .. }) = &spec.execution
        else {
            return Err(RuntimeError::new(
                tpt_runtime_core::error::ErrorKind::InvalidConfiguration,
                "demo backend only handles the windows process spec",
            ));
        };

        let child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        *self.pid.lock().unwrap() = Some(child.id());

        let (tx, rx) = oneshot::channel();
        let child = Arc::new(Mutex::new(child));

        // A watcher thread reports the final status exactly once.
        let watcher = Arc::clone(&child);
        std::thread::spawn(move || {
            let status = watcher.lock().unwrap().wait();
            let _ = tx.send(match status {
                Ok(s) if s.success() => ExitStatus::success(),
                Ok(s) => ExitStatus { code: s.code(), killed: false, failed: true },
                Err(_) => ExitStatus::terminated(),
            });
        });

        Ok(Box::new(DemoInstance {
            child,
            exit: Mutex::new(Some(rx)),
            stopped: Mutex::new(false),
        }))
    }
}

struct DemoInstance {
    child: Arc<Mutex<std::process::Child>>,
    exit: Mutex<Option<oneshot::Receiver<ExitStatus>>>,
    stopped: Mutex<bool>,
}

impl WorkloadInstance for DemoInstance {
    fn stats(&self) -> Result<ResourceUsage> {
        // A real backend samples job objects or engine counters here.
        Ok(ResourceUsage::default())
    }

    fn stop(&self, mode: StopMode) -> Result<()> {
        *self.stopped.lock().unwrap() = true;
        let mut child = self.child.lock().unwrap();
        match mode {
            StopMode::Graceful => { let _ = child.kill(); }
            StopMode::Kill => { let _ = child.kill(); }
        }
        Ok(())
    }

    fn exit(&self) -> oneshot::Receiver<ExitStatus> {
        // Take the receiver once; the manager calls this exactly once.
        self.exit.lock().unwrap().take().expect("exit() called twice")
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({ "pid": self.child.lock().unwrap().id() })
    }
}

// The manager accepts any Arc<dyn ExecutionBackend>.
fn register(manager: &tpt_runtime_workload::WorkloadManager) {
    manager.register_backend(DemoBackend::new());
}
```

## Log capture

Every backend funnels workload output into the same buffers, so `tpt logs`
behaves identically regardless of backend. `LogBuffer` is a bounded ring: a
runaway workload cannot exhaust memory.

```rust
use tpt_runtime_process::LogCapture;

let dir = std::env::temp_dir().join("log-capture-doc");
let capture = LogCapture::create(&dir, "stdout").unwrap();

capture.spawn_reader(std::process::Command::new("cmd")
    .args(["/C", "echo hello from the workload"])
    .stdout(std::process::Stdio::piped())
    .spawn()
    .unwrap()
    .stdout
    .expect("stdout was piped"));

// Give the reader thread a moment, then tail the buffer.
std::thread::sleep(std::time::Duration::from_millis(400));
let lines = capture.buffer.tail(10);
assert!(
    lines.iter().any(|l| l.contains("hello from the workload")),
    "captured lines: {lines:?}"
);
```

`LogBuffer` is usable on its own when a backend wants an in-memory-only sink:

```rust
use tpt_runtime_process::logs::LogBuffer;

let buffer = LogBuffer::new(3);          // keep only the last 3 lines
buffer.push("one");
buffer.push("two");
buffer.push("three");
buffer.push("four");

assert_eq!(buffer.len(), 3);
assert_eq!(buffer.tail(2), vec!["three".to_owned(), "four".to_owned()]);
```

## Contract details worth respecting

- `prepare` must be safe to call twice with the same arguments.
- `exit` is called exactly once per instance; the receiver is taken on first
  call. It **must** complete even when the workload is killed or crashes, or
  the manager will wait forever.
- `stats` is polled, so it should be cheap and must not block on I/O.
- Backends without graceful shutdown should treat `StopMode::Graceful` as
  termination and say so in their own documentation.

## Registering a backend

The manager consumes the trait, never a concrete backend type, which is what
keeps the workload model backend-independent.

```rust
use std::sync::Arc;

// At construction time, with the real backends:
fn register_all(manager: &tpt_runtime_workload::WorkloadManager) {
    manager.register_backend(Arc::new(tpt_runtime_windows::WindowsProcessBackend::new()));
    manager.register_backend(Arc::new(tpt_runtime_wasm::WasmBackend::new().unwrap()));
    manager.register_backend(Arc::new(tpt_runtime_oci::OciBackend::new("/var/lib/tpt/oci").unwrap()));
    manager.register_backend(Arc::new(tpt_runtime_linux::LinuxBackend::new()));
}
```

## Testing

```console
cargo test -p tpt-runtime-process
```

Covers `ExitStatus` constructors, mount resolution and `LogBuffer` ring
behaviour.
