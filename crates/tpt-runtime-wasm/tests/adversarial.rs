//! Adversarial WASM sandbox tests (SPEC §46): modules probing the sandbox —
//! environment scraping, filesystem escape attempts, unbounded memory
//! growth, foreign imports — are contained or fail loudly.
//!
//! The fixtures are WAT so each hostile behavior stays readable; wasmtime
//! accepts the text format anywhere a module is expected.

use std::path::PathBuf;
use std::time::Duration;
use tpt_runtime_capability::CapabilitySet;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_model::execution::{ExecutionSpec, WasmClass, WasmModuleSpec};
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_model::resources::ResourceSpec;
use tpt_runtime_model::volume::{VolumeAccessMode, VolumeMount};
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{
    ExecutionBackend, ExitStatus, ResolvedMount, StartContext, WorkloadInstance,
};

/// Scrapes the WASI environment: traps unless *exactly one* variable is
/// visible and it is the manifest-scoped `TPT_TEST=marker`.
const ENV_PROBE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "environ_sizes_get"
    (func $sizes (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "environ_get"
    (func $get (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 1024) "TPT_TEST=marker\00")
  (func (export "_start")
    (local $i i32)
    (if (i32.ne (call $sizes (i32.const 0) (i32.const 4)) (i32.const 0))
      (then unreachable))
    (if (i32.ne (i32.load (i32.const 0)) (i32.const 1))
      (then unreachable))
    (drop (call $get (i32.const 0) (i32.const 2048)))
    (loop $compare
      (br_if 1 (i32.ge_u (local.get $i) (i32.const 16)))
      (if (i32.ne
            (i32.load8_u (i32.add (i32.const 2048) (local.get $i)))
            (i32.load8_u (i32.add (i32.const 1024) (local.get $i))))
        (then unreachable))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $compare))))
"#;

/// Tries to escape the preopen: direct parent traversal, deep traversal
/// through a subdirectory, and a check that no fd exists beyond the one
/// preopen (least privilege). Traps if any escape succeeds.
const ESCAPE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_fdstat_get"
    (func $fdstat (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "../escaped.txt")
  (data (i32.const 64) "sub/../../../escaped.txt")
  (func (export "_start") (local $errno i32)
    (local.set $errno
      (call $path_open
        (i32.const 3) (i32.const 0)
        (i32.const 8) (i32.const 14)
        (i32.const 0)
        (i64.const 0) (i64.const 0)
        (i32.const 0) (i32.const 128)))
    (if (i32.eq (local.get $errno) (i32.const 0)) (then unreachable))
    (local.set $errno
      (call $path_open
        (i32.const 3) (i32.const 0)
        (i32.const 64) (i32.const 21)
        (i32.const 0)
        (i64.const 0) (i64.const 0)
        (i32.const 0) (i32.const 128)))
    (if (i32.eq (local.get $errno) (i32.const 0)) (then unreachable))
    (if (i32.eq (call $fdstat (i32.const 4) (i32.const 128)) (i32.const 0))
      (then unreachable))))
"#;

/// Attempts to create a file inside a read-only preopen; traps if the
/// write is permitted.
const RO_WRITE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "created.txt")
  (func (export "_start") (local $errno i32)
    (local.set $errno
      (call $path_open
        (i32.const 3) (i32.const 0)
        (i32.const 8) (i32.const 11)
        (i32.const 1)
        (i64.const 4) (i64.const 0)
        (i32.const 0) (i32.const 128)))
    (if (i32.eq (local.get $errno) (i32.const 0)) (then unreachable))))
"#;

/// Grows linear memory in 1 MiB steps forever.
const GROW_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "_start")
    (loop $grow
      (drop (memory.grow (i32.const 16)))
      (br $grow))))
"#;

/// Imports a non-WASI host function: instantiation must refuse it.
const FOREIGN_IMPORT_WAT: &str = r#"
(module
  (import "env" "mystery" (func $mystery))
  (func (export "_start") (call $mystery)))
"#;

const BUSY_LOOP_WAT: &str = r#"
(module
  (func (export "_start") (loop (br 0))))
"#;

fn backend() -> tpt_runtime_wasm::WasmBackend {
    tpt_runtime_wasm::WasmBackend::new().unwrap()
}

fn ctx(log_dir: PathBuf) -> StartContext {
    ctx_with_mounts(log_dir, vec![])
}

fn ctx_with_mounts(log_dir: PathBuf, mounts: Vec<ResolvedMount>) -> StartContext {
    StartContext {
        workload_id: WorkloadId::generate(),
        mounts,
        log_dir,
        capabilities: CapabilitySet::empty(),
        network_mode: NetworkMode::None,
        exposed_ports: vec![],
    }
}

fn spec(
    dir: &std::path::Path,
    file: &str,
    wat: &str,
    env: &[(&str, &str)],
    resources: ResourceSpec,
) -> WorkloadSpec {
    let module = dir.join(file);
    std::fs::write(&module, wat).unwrap();
    WorkloadSpec {
        name: "adversarial".to_owned(),
        execution: ExecutionSpec::WasmModule(WasmModuleSpec {
            module,
            class: WasmClass::Command,
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            args: vec![],
        }),
        resources,
        network: NetworkSpec::default(),
        volumes: Vec::<VolumeMount>::new(),
        devices: vec![],
        capabilities: vec![],
        labels: Default::default(),
    }
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpt-wasm-adv-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

fn run_to_exit(instance: &dyn WorkloadInstance) -> ExitStatus {
    runtime().block_on(async {
        let rx = instance.exit();
        rx.await.unwrap_or(ExitStatus::terminated())
    })
}

#[test]
fn environment_is_manifest_scoped_not_host_scoped() {
    let dir = workspace("env");
    let workload = spec(
        &dir,
        "probe.wasm",
        ENV_PROBE_WAT,
        &[("TPT_TEST", "marker")],
        ResourceSpec::default(),
    );

    let backend = backend();
    let context = ctx(dir.join("logs"));
    backend.prepare(&workload, &context).unwrap();
    let instance = backend.start(&workload, &context).unwrap();
    let status = run_to_exit(&*instance);

    assert_eq!(
        status.code,
        Some(0),
        "module trapped: host environment leaked into the sandbox or value mismatched"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn preopen_escape_attempts_are_denied() {
    let dir = workspace("escape");
    let volume = dir.join("volume");
    std::fs::create_dir_all(&volume).unwrap();
    // The escape target exists outside the preopen, so a successful
    // traversal would open it and trip the module's trap.
    std::fs::write(dir.join("escaped.txt"), "host content").unwrap();

    let workload = spec(
        &dir,
        "escape.wasm",
        ESCAPE_WAT,
        &[],
        ResourceSpec::default(),
    );
    let context = ctx_with_mounts(
        dir.join("logs"),
        vec![ResolvedMount {
            name: "data".to_owned(),
            mount: "/data".to_owned(),
            host_path: volume.clone(),
            mode: VolumeAccessMode::ReadWrite,
        }],
    );

    let backend = backend();
    backend.prepare(&workload, &context).unwrap();
    let instance = backend.start(&workload, &context).unwrap();
    let status = run_to_exit(&*instance);

    assert_eq!(
        status.code,
        Some(0),
        "module trapped: a filesystem escape or fd-leak check failed"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("escaped.txt")).unwrap(),
        "host content",
        "the escaped file must be untouched"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn read_only_volume_rejects_writes() {
    let dir = workspace("ro");
    let volume = dir.join("volume");
    std::fs::create_dir_all(&volume).unwrap();

    let workload = spec(&dir, "ro.wasm", RO_WRITE_WAT, &[], ResourceSpec::default());
    let context = ctx_with_mounts(
        dir.join("logs"),
        vec![ResolvedMount {
            name: "data".to_owned(),
            mount: "/data".to_owned(),
            host_path: volume.clone(),
            mode: VolumeAccessMode::ReadOnly,
        }],
    );

    let backend = backend();
    backend.prepare(&workload, &context).unwrap();
    let instance = backend.start(&workload, &context).unwrap();
    let status = run_to_exit(&*instance);

    assert_eq!(
        status.code,
        Some(0),
        "module trapped: a write into a read-only preopen succeeded"
    );
    assert!(
        !volume.join("created.txt").exists(),
        "read-only volume must not gain files"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unbounded_memory_growth_is_bounded_by_fuel() {
    let dir = workspace("grow");
    let workload = spec(
        &dir,
        "grow.wasm",
        GROW_WAT,
        &[],
        ResourceSpec {
            fuel: Some(50_000),
            ..Default::default()
        },
    );

    let backend = backend();
    let context = ctx(dir.join("logs"));
    let instance = backend.start(&workload, &context).unwrap();
    let status = run_to_exit(&*instance);

    assert!(status.failed, "runaway memory growth is a failure");
    assert!(status.killed, "runtime-interrupted growth is a kill");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn foreign_imports_fail_instantiation() {
    let dir = workspace("foreign");
    let workload = spec(
        &dir,
        "foreign.wasm",
        FOREIGN_IMPORT_WAT,
        &[],
        ResourceSpec::default(),
    );

    let backend = backend();
    let context = ctx(dir.join("logs"));
    // Compilation succeeds (imports are resolved at instantiation), so
    // prepare passes...
    backend.prepare(&workload, &context).unwrap();
    // ...but the sandbox provides no `env` namespace: start must fail loudly.
    let err = match backend.start(&workload, &context) {
        Err(err) => err,
        Ok(_) => panic!("foreign import must fail instantiation"),
    };
    assert!(err.message.contains("instantiation"), "err: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn wall_clock_timeout_kills_compute_bound_module() {
    let dir = workspace("timeout");
    let workload = spec(
        &dir,
        "busy.wasm",
        BUSY_LOOP_WAT,
        &[],
        ResourceSpec {
            timeout_secs: Some(1),
            ..Default::default()
        },
    );

    let backend = backend();
    let context = ctx(dir.join("logs"));
    let instance = backend.start(&workload, &context).unwrap();
    let started = std::time::Instant::now();
    let status = run_to_exit(&*instance);
    let elapsed = started.elapsed();

    assert!(status.failed, "timeout is a failure");
    assert!(status.killed, "timeout is a runtime kill, not a crash");
    assert!(
        elapsed < Duration::from_secs(15),
        "timeout enforced in {:?}, expected ~1s",
        elapsed
    );
    std::fs::remove_dir_all(&dir).ok();
}
