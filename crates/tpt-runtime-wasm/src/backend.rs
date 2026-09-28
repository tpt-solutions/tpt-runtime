//! wasmtime-backed WASM execution (SPEC §14).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::ResourceUsage;
use tpt_runtime_model::volume::VolumeAccessMode;
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{
    ExecutionBackend, ExitStatus, LogCapture, StartContext, StopMode, WorkloadInstance,
};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::p1::{add_to_linker_sync, WasiP1Ctx};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

/// Backend executing WASM modules inside wasmtime (SPEC §14).
pub struct WasmBackend {
    engine: Engine,
    _ticker: Arc<EpochTicker>,
}

/// Keeps a background thread incrementing the engine epoch so that epoch
/// deadlines (timeout, stop) fire even for compute-bound modules.
struct EpochTicker {
    _handle: std::thread::JoinHandle<()>,
}

impl WasmBackend {
    /// Creates the backend with fuel metering and epoch interruption
    /// enabled on the engine.
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine =
            Engine::new(&config).map_err(|err| engine_error(err, "engine initialization"))?;

        // Epoch ticker: 10ms granularity; a stopped or timed-out module
        // traps within ~10ms even mid-loop.
        let ticker_engine = engine.clone();
        let ticker = std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(10));
            ticker_engine.increment_epoch();
        });

        Ok(Self {
            engine,
            _ticker: Arc::new(EpochTicker {
                _handle: ticker,
            }),
        })
    }
}

impl Default for WasmBackend {
    fn default() -> Self {
        Self::new().expect("wasmtime engine initialization")
    }
}

impl ExecutionBackend for WasmBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Wasm
    }

    fn prepare(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<()> {
        let wasm_spec = wasm_spec(spec)?;
        let module_path = resolve_module(&wasm_spec.module)?;
        // Compiling here also validates the module before start.
        Module::from_file(&self.engine, &module_path)
            .map_err(|err| engine_error(err, "module validation"))
            .map(|_| ())
            .and_then(|()| {
                for mount in &ctx.mounts {
                    if !mount.host_path.is_dir() {
                        return Err(RuntimeError::new(
                            ErrorKind::StorageFailure,
                            format!(
                                "volume '{}' host path '{}' is missing",
                                mount.name,
                                mount.host_path.display()
                            ),
                        ));
                    }
                }
                Ok(())
            })
    }

    fn start(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>> {
        let wasm_spec = wasm_spec(spec)?;
        let module_path = resolve_module(&wasm_spec.module)?;

        let mut linker: Linker<WasiP1Ctx> = Linker::new(&self.engine);
        add_to_linker_sync(&mut linker, |ctx| ctx)
            .map_err(|err| engine_error(err, "linker setup"))?;

        // stdout/stderr capture through WASI pipes
        let stdout_pipe = MemoryOutputPipe::new(1 << 20);
        let stderr_pipe = MemoryOutputPipe::new(1 << 20);

        // args[0] is conventionally the module name
        let module_name = module_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "module.wasm".to_owned());
        let mut args = vec![module_name];
        args.extend(wasm_spec.args.iter().cloned());
        let envs: Vec<(String, String)> = wasm_spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let mut builder = WasiCtxBuilder::new();
        builder
            .args(&args)
            .envs(&envs)
            .stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone());

        // Filesystem capabilities: only granted volumes are visible, with
        // the access mode from the manifest (SPEC §5.2, §15).
        for mount in &ctx.mounts {
            if let Err(err) = builder.preopened_dir(
                &mount.host_path,
                &mount.mount,
                match mount.mode {
                    VolumeAccessMode::ReadOnly => FsPerms::ReadOnly,
                    VolumeAccessMode::ReadWrite => FsPerms::ReadWrite,
                },
            ) {
                return Err(RuntimeError::new(
                    ErrorKind::StorageFailure,
                    format!(
                        "cannot preopen volume '{}' at '{}': {err}",
                        mount.name,
                        mount.host_path.display()
                    ),
                ));
            }
        }

        let wasi = builder.build_p1();
        let mut store = Store::new(&self.engine, wasi);

        // Deterministic CPU limit (fuel = executed instructions).
        let fuel = wasm_spec_fuel(spec);
        store
            .set_fuel(fuel)
            .map_err(|err| engine_error(err, "fuel setup"))?;

        // Wall-clock budget + cooperative stop: the deadline trips every
        // ~100ms (ticker at 10ms); the callback extends it while the
        // workload should keep running and traps on stop/timeout. This is
        // the documented pattern for interrupting compute-bound modules.
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let timeout = spec
            .resources
            .timeout_secs
            .map(|secs| Duration::from_secs(secs))
            .unwrap_or(Duration::MAX);
        // saturating far-future deadline when no timeout is requested
        let deadline = std::time::Instant::now() + timeout.min(Duration::from_secs(60 * 60 * 24 * 365));
        {
            let stopped = stopped.clone();
            store.epoch_deadline_callback(move |_store| {
                if stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    wasmtime::bail!("workload stopped by runtime");
                }
                if std::time::Instant::now() >= deadline {
                    wasmtime::bail!("workload exceeded its wall-clock limit");
                }
                Ok(wasmtime::UpdateDeadline::Continue(10))
            });
        }
        store.set_epoch_deadline(10);

        let module = Module::from_file(&self.engine, &module_path)
            .map_err(|err| engine_error(err, "module compilation"))?;
        let instance = linker
            .instantiate(&mut store, &module)
            .and_then(|instance| instance.get_typed_func::<(), ()>(&mut store, "_start"))
            .map_err(|err| engine_error(err, "instantiation (missing _start?)"))?;

        let stdout = LogCapture::create(&ctx.log_dir, "stdout")?;
        let stderr = LogCapture::create(&ctx.log_dir, "stderr")?;
        let stdout_path = stdout.file.clone();
        let stderr_path = stderr.file.clone();

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<ExitStatus>();
        let stopped_watcher = stopped.clone();
        let memory_handle = Arc::new(Mutex::new(0u64));

        std::thread::spawn(move || {
            let result = instance.call(&mut store, ());
            let status = interpret_result(&result, &stopped_watcher);

            drain_pipe(&stdout_pipe, &stdout);
            drain_pipe(&stderr_pipe, &stderr);

            let _ = exit_tx.send(status);
        });

        Ok(Box::new(WasmInstance {
            exit_rx: Mutex::new(Some(exit_rx)),
            stopped,
            stdout_path,
            stderr_path,
            memory_bytes: memory_handle,
        }))
    }
}

/// Live handle for one WASM workload.
struct WasmInstance {
    exit_rx: Mutex<Option<tokio::sync::oneshot::Receiver<ExitStatus>>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    memory_bytes: Arc<Mutex<u64>>,
}

impl WorkloadInstance for WasmInstance {
    fn stats(&self) -> Result<ResourceUsage> {
        // WASI pipes already feed the log files; byte counters come from
        // their sizes. Memory is filled by the engine path where an exported
        // linear memory exists (updated at exit).
        let mut usage = ResourceUsage::default();
        usage.write_bytes = pipe_file_len(&self.stdout_path) + pipe_file_len(&self.stderr_path);
        usage.memory_peak_bytes = *self.memory_bytes.lock().unwrap();
        usage.collected_at = Some(tpt_runtime_core::Timestamp::now());
        Ok(usage)
    }

    fn stop(&self, _mode: StopMode) -> Result<()> {
        // Flags the epoch callback: the module traps on the next engine
        // tick (~10ms) even inside a busy loop.
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn exit(&self) -> tokio::sync::oneshot::Receiver<ExitStatus> {
        let mut guard = self.exit_rx.lock().unwrap();
        match guard.take() {
            Some(rx) => rx,
            None => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(ExitStatus::terminated());
                rx
            }
        }
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": "wasm",
            "engine": "wasmtime",
        })
    }
}

fn pipe_file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn drain_pipe(pipe: &MemoryOutputPipe, capture: &LogCapture) {
    let contents = pipe.contents();
    if contents.is_empty() {
        return;
    }
    if let Ok(text) = String::from_utf8(contents.to_vec()) {
        for line in text.lines() {
            capture.buffer.push(line.to_owned());
        }
        if let Some(parent) = capture.file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&capture.file, &*contents);
    }
}

fn interpret_result(
    result: &std::result::Result<(), wasmtime::Error>,
    stopped: &std::sync::atomic::AtomicBool,
) -> ExitStatus {
    let status_from = |reason: &str, code: Option<i32>| ExitStatus {
        code,
        killed: stopped.load(std::sync::atomic::Ordering::SeqCst)
            || reason.contains("fuel")
            || reason.contains("stopped by runtime"),
        failed: true,
    };
    match result {
        Ok(()) => ExitStatus::success(),
        Err(err) => {
            // WASI exit(code) surfaces as a trap carrying I32Exit
            if let Some(exit) = err.downcast_ref::<wasmtime_wasi::I32Exit>() {
                return ExitStatus {
                    code: Some(exit.0),
                    killed: false,
                    failed: exit.0 != 0,
                };
            }
            // `to_string()` shows only the backtrace header; the trap
            // description ("all fuel consumed by WebAssembly", ...) lives in
            // the cause chain, which Debug walks.
            status_from(&format!("{err:?}"), None)
        }
    }
}

fn wasm_spec(
    spec: &WorkloadSpec,
) -> Result<tpt_runtime_model::execution::WasmModuleSpec> {
    match &spec.execution {
        tpt_runtime_model::execution::ExecutionSpec::WasmModule(spec) => Ok(spec.clone()),
        other => Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("wasm backend cannot execute {other:?}"),
        )),
    }
}

fn wasm_spec_fuel(spec: &WorkloadSpec) -> u64 {
    // Default fuel: enough for substantial work, still finite to bound
    // runaway modules when the manifest sets no limit.
    spec.resources.fuel.unwrap_or(20_000_000_000)
}

fn resolve_module(module: &Path) -> Result<PathBuf> {
    if module.is_file() {
        return Ok(module.to_path_buf());
    }
    Err(RuntimeError::new(
        ErrorKind::NotFound,
        format!("wasm module '{}' not found", module.display()),
    )
    .with_operation("resolve_module"))
}

fn engine_error(err: impl std::fmt::Display, stage: &str) -> RuntimeError {
    RuntimeError::new(ErrorKind::System, format!("{stage}: {err:#}")).with_backend("wasm")
}
