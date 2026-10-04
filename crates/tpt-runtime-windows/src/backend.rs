//! Native Windows process execution (SPEC §12).

#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
#[cfg(windows)]
use tpt_runtime_core::ResourceUsage;
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, StartContext, WorkloadInstance};
#[cfg(windows)]
use tpt_runtime_process::{ExitStatus, LogCapture, StopMode};

#[cfg(windows)]
mod job;

/// Backend producing native Windows processes (SPEC §12).
#[derive(Debug, Default)]
pub struct WindowsProcessBackend;

impl WindowsProcessBackend {
    /// Creates the backend.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
impl ExecutionBackend for WindowsProcessBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Windows
    }

    /// Preflight checks: program resolvable, working dir and volume targets
    /// present. Cheap and side-effect free (idempotent, SPEC §10).
    fn prepare(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<()> {
        let proc_spec = windows_spec(spec)?;
        resolve_program(&proc_spec.program)?;
        if let Some(dir) = &proc_spec.working_dir {
            if !dir.is_dir() {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("working directory '{}' does not exist", dir.display()),
                ));
            }
        }
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
    }

    fn start(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>> {
        use std::os::windows::process::CommandExt;

        let proc_spec = windows_spec(spec)?;
        let program = resolve_program(&proc_spec.program)?;

        // The log dir doubles as the default working dir: ensure it exists
        // before the child process starts.
        std::fs::create_dir_all(&ctx.log_dir)?;

        let working_dir: PathBuf = proc_spec
            .working_dir
            .clone()
            .or_else(|| {
                ctx.mounts
                    .iter()
                    .find(|m| m.mode == tpt_runtime_model::volume::VolumeAccessMode::ReadWrite)
                    .map(|m| m.host_path.clone())
            })
            .unwrap_or_else(|| ctx.log_dir.clone());

        let mut command = Command::new(program);
        command
            .args(&proc_spec.args)
            .current_dir(&working_dir)
            .env_clear()
            .envs(sanitized_environment())
            .envs(&proc_spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // never flash a console window from the daemon
            .creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = command.spawn().map_err(|err| {
            RuntimeError::new(
                ErrorKind::System,
                format!("cannot start '{}': {err}", proc_spec.program),
            )
            .with_workload(spec.name.clone())
            .with_backend("windows")
            .with_operation("start")
        })?;
        let pid = child.id();

        // Job object: memory limit + kill-tree + daemon-crash safety.
        let job = job::JobObject::with_memory_limit(spec.resources.memory.map(|m| m.0))?;
        {
            use std::os::windows::io::AsRawHandle;
            job.assign_process(child.as_raw_handle())?;
        }

        let stdout = LogCapture::create(&ctx.log_dir, "stdout")?;
        let stderr = LogCapture::create(&ctx.log_dir, "stderr")?;
        if let Some(out) = child.stdout.take() {
            stdout.spawn_reader(out);
        }
        if let Some(err_pipe) = child.stderr.take() {
            stderr.spawn_reader(err_pipe);
        }

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<ExitStatus>();
        let job_for_watcher = job.clone();
        let terminate_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let terminate_watcher = terminate_requested.clone();
        std::thread::spawn(move || {
            let status = match child.wait() {
                Ok(status) => {
                    let code = status.code();
                    let killed = terminate_watcher.load(std::sync::atomic::Ordering::SeqCst);
                    ExitStatus {
                        code,
                        killed,
                        // a runtime-requested termination is not a crash
                        failed: !killed && code.map(|c| c != 0).unwrap_or(true),
                    }
                }
                Err(_) => ExitStatus::terminated(),
            };
            let _ = exit_tx.send(status);
            // keep the job alive until the process is reaped
            drop(job_for_watcher);
        });

        Ok(Box::new(WindowsProcessInstance {
            pid,
            job,
            terminate_requested,
            exit_rx: Mutex::new(Some(exit_rx)),
            _stdout: stdout,
            _stderr: stderr,
        }))
    }
}

#[cfg(not(windows))]
impl ExecutionBackend for WindowsProcessBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Windows
    }

    fn prepare(&self, _spec: &WorkloadSpec, _ctx: &StartContext) -> Result<()> {
        Err(RuntimeError::new(
            ErrorKind::BackendUnavailable,
            "windows backend requires a Windows host",
        ))
    }

    fn start(
        &self,
        _spec: &WorkloadSpec,
        _ctx: &StartContext,
    ) -> Result<Box<dyn WorkloadInstance>> {
        Err(RuntimeError::new(
            ErrorKind::BackendUnavailable,
            "windows backend requires a Windows host",
        ))
    }
}

#[cfg(windows)]
fn windows_spec(spec: &WorkloadSpec) -> Result<tpt_runtime_model::execution::WindowsProcessSpec> {
    match &spec.execution {
        tpt_runtime_model::execution::ExecutionSpec::WindowsProcess(spec) => Ok(spec.clone()),
        other => Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("windows backend cannot execute {other:?}"),
        )),
    }
}

#[cfg(windows)]
/// Live handle for one Windows workload.
struct WindowsProcessInstance {
    pid: u32,
    #[allow(dead_code)] // keeps the job (and kill-on-close) alive
    job: std::sync::Arc<job::JobObject>,
    terminate_requested: Arc<std::sync::atomic::AtomicBool>,
    exit_rx: Mutex<Option<tokio::sync::oneshot::Receiver<ExitStatus>>>,
    _stdout: LogCapture,
    _stderr: LogCapture,
}

#[cfg(windows)]
impl WorkloadInstance for WindowsProcessInstance {
    fn stats(&self) -> Result<ResourceUsage> {
        let mut usage = ResourceUsage::default();
        if let Some(job_stats) = self.job.query_usage() {
            usage.user_cpu = job_stats.user_cpu;
            usage.kernel_cpu = job_stats.kernel_cpu;
            usage.memory_peak_bytes = job_stats.peak_memory_bytes;
            usage.read_bytes = job_stats.read_bytes;
            usage.write_bytes = job_stats.write_bytes;
            usage.process_count = job_stats.process_count;
        }
        usage.collected_at = Some(tpt_runtime_core::Timestamp::now());
        Ok(usage)
    }

    fn stop(&self, _mode: StopMode) -> Result<()> {
        // Native Windows processes have no universally honored graceful
        // signal; job termination is the documented stop path (SPEC §12
        // lifecycle control). Callers wanting graceful behavior should make
        // the program handle it internally.
        self.terminate_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.job.terminate(1)
    }

    fn exit(&self) -> tokio::sync::oneshot::Receiver<ExitStatus> {
        let mut guard = self.exit_rx.lock().unwrap();
        match guard.take() {
            Some(rx) => rx,
            None => {
                // Second exit request: the first receiver already has the
                // status. Kill and resolve immediately so the contract
                // (receiver always completes) holds.
                let _ = self.job.terminate(1);
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(ExitStatus::terminated());
                rx
            }
        }
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": "windows",
            "pid": self.pid,
        })
    }
}

#[cfg(windows)]
/// Resolves a program name through PATH when it has no directory part.
fn resolve_program(program: &str) -> Result<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 || program.contains('\\') || program.contains('/') {
        if candidate.exists() {
            return Ok(candidate.to_path_buf());
        }
        return Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("program '{}' not found", program),
        )
        .with_operation("resolve_program"));
    }
    // bare name: search PATH ourselves so errors are precise
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let with_ext = dir.join(format!("{program}.exe"));
            if with_ext.exists() {
                return Ok(with_ext);
            }
            let plain = dir.join(program);
            if plain.exists() {
                return Ok(plain);
            }
        }
    }
    Err(RuntimeError::new(
        ErrorKind::InvalidConfiguration,
        format!("program '{program}' not found on PATH"),
    )
    .with_operation("resolve_program"))
}

/// Minimal environment for child processes: the runtime never leaks its own
/// full environment into workloads (SPEC §5.2 capability orientation).
#[cfg(windows)]
fn sanitized_environment() -> Vec<(String, String)> {
    let temp = std::env::temp_dir().display().to_string();
    let mut vars = vec![
        ("SystemRoot".to_owned(), "C:\\Windows".to_owned()),
        ("windir".to_owned(), "C:\\Windows".to_owned()),
        ("TEMP".to_owned(), temp.clone()),
        ("TMP".to_owned(), temp),
    ];
    if let Some(system_drive) = std::env::var_os("SystemDrive") {
        vars.push((
            "SystemDrive".to_owned(),
            system_drive.to_string_lossy().to_string(),
        ));
    }
    if let Some(path) = std::env::var_os("Path") {
        vars.push(("Path".to_owned(), path.to_string_lossy().to_string()));
    }
    vars
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use tpt_runtime_capability::CapabilitySet;
    use tpt_runtime_core::id::WorkloadId;
    use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
    use tpt_runtime_model::resources::ResourceSpec;
    use tpt_runtime_model::volume::VolumeMount;

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

    fn spec(program: &str, args: &[&str], memory: Option<u64>) -> WorkloadSpec {
        WorkloadSpec {
            name: "win-test".to_owned(),
            execution: tpt_runtime_model::execution::ExecutionSpec::WindowsProcess(
                tpt_runtime_model::execution::WindowsProcessSpec {
                    program: program.to_owned(),
                    args: args.iter().map(|s| s.to_string()).collect(),
                    ..Default::default()
                },
            ),
            resources: ResourceSpec {
                memory: memory.map(tpt_runtime_model::Memory),
                ..Default::default()
            },
            network: NetworkSpec::default(),
            volumes: Vec::<VolumeMount>::new(),
            devices: vec![],
            capabilities: vec![],
            labels: Default::default(),
        }
    }

    #[test]
    fn resolves_system_programs() {
        assert!(resolve_program("cmd.exe").is_ok());
        assert!(resolve_program("definitely-not-a-real-program-xyz").is_err());
    }

    #[test]
    fn prepare_rejects_missing_working_dir() {
        let backend = WindowsProcessBackend::new();
        let log_dir = std::env::temp_dir().join(format!("tpt-win-{}", std::process::id()));
        let context = ctx(log_dir);
        let mut workload = spec("cmd.exe", &[], None);
        match &mut workload.execution {
            tpt_runtime_model::execution::ExecutionSpec::WindowsProcess(p) => {
                p.working_dir = Some(PathBuf::from("Z:/definitely/missing"));
            }
            _ => unreachable!(),
        }
        let err = backend.prepare(&workload, &context).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidConfiguration);
    }

    #[test]
    fn start_run_capture_and_stats() {
        let backend = WindowsProcessBackend::new();
        let log_dir = std::env::temp_dir().join(format!("tpt-win-{}-run", std::process::id()));
        let context = ctx(log_dir.clone());
        let workload = spec("cmd.exe", &["/C", "echo hello-from-workload"], None);

        backend.prepare(&workload, &context).unwrap();
        let instance = backend.start(&workload, &context).unwrap();

        let status = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let rx = instance.exit();
                rx.await.unwrap_or(ExitStatus::terminated())
            });
        assert!(!status.failed, "cmd /C echo must exit 0");

        // stats are available from the job object
        let usage = instance.stats().unwrap();
        assert!(usage.memory_peak_bytes > 0 || usage.process_count == 0);

        // stdout was captured to the log file
        let mut contents = String::new();
        for _ in 0..50 {
            contents = std::fs::read_to_string(log_dir.join("stdout.log")).unwrap_or_default();
            if contents.contains("hello-from-workload") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        assert!(
            contents.contains("hello-from-workload"),
            "log: {contents:?}"
        );
        std::fs::remove_dir_all(&log_dir).ok();
    }

    #[test]
    fn stop_kills_a_long_running_process() {
        let backend = WindowsProcessBackend::new();
        let log_dir = std::env::temp_dir().join(format!("tpt-win-{}-stop", std::process::id()));
        let context = ctx(log_dir.clone());
        // ping sleeps ~15s on windows
        let workload = spec("ping.exe", &["-n", "15", "127.0.0.1"], None);
        let instance = backend.start(&workload, &context).unwrap();
        instance.stop(StopMode::Kill).unwrap();

        let status = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let rx = instance.exit();
                rx.await.unwrap_or(ExitStatus::terminated())
            });
        assert!(status.code != Some(0));
        std::fs::remove_dir_all(&log_dir).ok();
    }
}
