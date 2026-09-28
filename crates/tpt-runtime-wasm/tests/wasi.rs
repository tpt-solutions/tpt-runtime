//! Integration tests: WASM execution through the full backend path.

use std::path::PathBuf;
use tpt_runtime_capability::CapabilitySet;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_model::execution::{ExecutionSpec, WasmClass, WasmModuleSpec};
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_model::resources::ResourceSpec;
use tpt_runtime_model::volume::VolumeMount;
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, ExitStatus, StartContext};

/// WAT text is accepted anywhere `.wasm` modules are: wasmtime detects the
/// text format, which lets the tests ship readable fixtures.
const HELLO_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "hello from wasm\n")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 16))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 24))))
)
"#;

const BUSY_LOOP_WAT: &str = r#"
(module
  (func (export "_start") (loop (br 0)))
)
"#;

fn backend() -> tpt_runtime_wasm::WasmBackend {
    tpt_runtime_wasm::WasmBackend::new().unwrap()
}

fn ctx(log_dir: PathBuf) -> StartContext {
    StartContext {
        workload_id: WorkloadId::generate(),
        mounts: vec![],
        log_dir,
        capabilities: CapabilitySet::empty(),
        network_mode: NetworkMode::None,
        exposed_ports: vec![],
    }
}

fn spec(dir: &std::path::Path, file: &str, wat: &str, fuel: Option<u64>) -> WorkloadSpec {
    let module = dir.join(file);
    std::fs::write(&module, wat).unwrap();
    WorkloadSpec {
        name: "wasm-test".to_owned(),
        execution: ExecutionSpec::WasmModule(WasmModuleSpec {
            module,
            class: WasmClass::Command,
            env: Default::default(),
            args: vec![],
        }),
        resources: ResourceSpec {
            fuel,
            ..Default::default()
        },
        network: NetworkSpec::default(),
        volumes: Vec::<VolumeMount>::new(),
        devices: vec![],
        capabilities: vec![],
        labels: Default::default(),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

#[test]
fn hello_module_runs_and_captures_stdout() {
    let dir = std::env::temp_dir().join(format!("tpt-wasm-hello-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let workload = spec(&dir, "hello.wasm", HELLO_WAT, None);

    let backend = backend();
    let context = ctx(dir.join("logs"));
    backend.prepare(&workload, &context).unwrap();
    let instance = backend.start(&workload, &context).unwrap();

    let status = runtime().block_on(async {
        let rx = instance.exit();
        rx.await.unwrap_or(ExitStatus::terminated())
    });
    assert_eq!(status.code, Some(0), "clean exit expected");
    assert!(!status.failed);

    let stdout = std::fs::read_to_string(context.log_dir.join("stdout.log")).unwrap_or_default();
    assert!(stdout.contains("hello from wasm"), "stdout: {stdout:?}");

    let usage = instance.stats().unwrap();
    assert!(usage.collected_at.is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fuel_limit_stops_infinite_loop() {
    let dir = std::env::temp_dir().join(format!("tpt-wasm-fuel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let workload = spec(&dir, "busy.wasm", BUSY_LOOP_WAT, Some(10_000));

    let backend = backend();
    let context = ctx(dir.join("logs"));
    let instance = backend.start(&workload, &context).unwrap();

    let status = runtime().block_on(async {
        let rx = instance.exit();
        rx.await.unwrap_or(ExitStatus::terminated())
    });
    assert!(status.failed, "fuel exhaustion is a failure");
    assert!(status.killed, "runtime-interrupted workload is killed");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stop_requests_trap_running_module() {
    let dir = std::env::temp_dir().join(format!("tpt-wasm-stop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // no fuel limit: the module would run ~forever without stop()
    let workload = spec(&dir, "busy.wasm", BUSY_LOOP_WAT, None);

    let backend = backend();
    let context = ctx(dir.join("logs"));
    let instance = backend.start(&workload, &context).unwrap();
    instance.stop(tpt_runtime_process::StopMode::Kill).unwrap();

    let status = runtime().block_on(async {
        let rx = instance.exit();
        rx.await.unwrap_or(ExitStatus::terminated())
    });
    assert!(status.killed, "stopped workload reports killed");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn prepare_rejects_missing_module() {
    let dir = std::env::temp_dir().join(format!("tpt-wasm-miss-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut workload = spec(&dir, "ghost.wasm", HELLO_WAT, None);
    std::fs::remove_file(dir.join("ghost.wasm")).unwrap();
    match &mut workload.execution {
        ExecutionSpec::WasmModule(w) => w.module = dir.join("ghost.wasm"),
        _ => unreachable!(),
    }
    let backend = backend();
    let context = ctx(dir.join("logs"));
    let err = backend.prepare(&workload, &context).unwrap_err();
    assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::NotFound);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn invalid_module_fails_prepare() {
    let dir = std::env::temp_dir().join(format!("tpt-wasm-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let workload = spec(&dir, "bad.wasm", "this is not wat or wasm", None);
    let backend = backend();
    let context = ctx(dir.join("logs"));
    assert!(backend.prepare(&workload, &context).is_err());
    std::fs::remove_dir_all(&dir).ok();
}
