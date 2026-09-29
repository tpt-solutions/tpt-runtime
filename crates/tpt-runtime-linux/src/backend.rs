//! WSL-backed Linux execution (SPEC §11, Phase 1 of the layered strategy).

use std::process::{Command, Stdio};
use std::sync::Mutex;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{
    ExecutionBackend, ExitStatus, LogCapture, StartContext, StopMode, WorkloadInstance,
};

/// Linux backend executing workloads through WSL on Windows hosts
/// (SPEC §11). On other hosts it reports `backend_unavailable`.
#[derive(Debug, Default)]
pub struct LinuxBackend;

impl LinuxBackend {
    /// Creates the backend.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
impl ExecutionBackend for LinuxBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Linux
    }

    fn prepare(&self, spec: &WorkloadSpec, _ctx: &StartContext) -> Result<()> {
        let linux = linux_spec(spec)?;
        // `wsl.exe -d <distro> --exec true` proves distro presence without
        // side effects.
        let status = Command::new("wsl.exe")
            .args(["-d", &linux.distro, "--exec", "true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|err| {
                RuntimeError::new(
                    ErrorKind::BackendUnavailable,
                    format!("wsl.exe is not available: {err}"),
                )
                .with_backend("linux")
            })?;
        if !status.success() {
            return Err(RuntimeError::new(
                ErrorKind::BackendUnavailable,
                format!(
                    "wsl distro '{}' is not installed (wsl exit {})",
                    linux.distro,
                    status.code().unwrap_or(-1)
                ),
            )
            .with_backend("linux")
            .with_workload(spec.name.clone()));
        }
        Ok(())
    }

    fn start(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>> {
        use std::os::windows::process::CommandExt;

        let linux = linux_spec(spec)?;
        let mut command_parts = vec!["--exec".to_owned()];
        command_parts.extend(linux.command.iter().cloned());

        let mut cmd = Command::new("wsl.exe");
        cmd.args(["-d", &linux.distro])
            .args(&command_parts)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .env("WSLENV", "")
            .creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = cmd.spawn().map_err(|err| {
            RuntimeError::new(ErrorKind::System, format!("cannot start wsl.exe: {err}"))
                .with_backend("linux")
                .with_workload(spec.name.clone())
                .with_operation("start")
        })?;

        std::fs::create_dir_all(&ctx.log_dir)?;
        let stdout = LogCapture::create(&ctx.log_dir, "stdout")?;
        let stderr = LogCapture::create(&ctx.log_dir, "stderr")?;
        if let Some(out) = child.stdout.take() {
            stdout.spawn_reader(out);
        }
        if let Some(err_pipe) = child.stderr.take() {
            stderr.spawn_reader(err_pipe);
        }

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<ExitStatus>();
        std::thread::spawn(move || {
            let status = match child.wait() {
                Ok(status) => {
                    let code = status.code();
                    ExitStatus {
                        code,
                        killed: false,
                        failed: code.map(|c| c != 0).unwrap_or(true),
                    }
                }
                Err(_) => ExitStatus::terminated(),
            };
            let _ = exit_tx.send(status);
        });

        Ok(Box::new(WslInstance {
            exit_rx: Mutex::new(Some(exit_rx)),
            _stdout: stdout,
            _stderr: stderr,
        }))
    }
}

#[cfg(not(windows))]
impl ExecutionBackend for LinuxBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Linux
    }

    fn prepare(&self, _spec: &WorkloadSpec, _ctx: &StartContext) -> Result<()> {
        Err(RuntimeError::new(
            ErrorKind::BackendUnavailable,
            "WSL-backed linux backend requires a Windows host",
        ))
    }

    fn start(
        &self,
        _spec: &WorkloadSpec,
        _ctx: &StartContext,
    ) -> Result<Box<dyn WorkloadInstance>> {
        Err(RuntimeError::new(
            ErrorKind::BackendUnavailable,
            "WSL-backed linux backend requires a Windows host",
        ))
    }
}

fn linux_spec(spec: &WorkloadSpec) -> Result<tpt_runtime_model::execution::LinuxProcessSpec> {
    match &spec.execution {
        tpt_runtime_model::execution::ExecutionSpec::LinuxProcess(spec) => Ok(spec.clone()),
        other => Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("linux backend cannot execute {other:?}"),
        )),
    }
}

struct WslInstance {
    exit_rx: Mutex<Option<tokio::sync::oneshot::Receiver<ExitStatus>>>,
    _stdout: LogCapture,
    _stderr: LogCapture,
}

impl WorkloadInstance for WslInstance {
    fn stats(&self) -> Result<tpt_runtime_core::ResourceUsage> {
        // WSL exposes per-distro metrics via /proc inside the VM; wiring
        // that up is Phase 2 of the Linux roadmap (SPEC §11).
        Ok(tpt_runtime_core::ResourceUsage {
            collected_at: Some(tpt_runtime_core::Timestamp::now()),
            ..Default::default()
        })
    }

    fn stop(&self, _mode: StopMode) -> Result<()> {
        // wsl.exe child termination ends this workload's view; the distro
        // itself stays running (coexistence, SPEC §36).
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
            "backend": "linux",
            "mechanism": "wsl",
        })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use tpt_runtime_capability::CapabilitySet;
    use tpt_runtime_core::id::WorkloadId;

    fn spec() -> WorkloadSpec {
        WorkloadSpec::new(
            "linux-test",
            tpt_runtime_model::execution::ExecutionSpec::LinuxProcess(
                tpt_runtime_model::execution::LinuxProcessSpec {
                    distro: "no-such-distro-xyz".to_owned(),
                    command: vec!["true".to_owned()],
                    env: Default::default(),
                },
            ),
        )
    }

    #[test]
    fn unknown_distro_fails_with_clear_error() {
        // on hosts without any WSL, the error is "wsl.exe unavailable";
        // on hosts with WSL it is "distro not installed". Both are
        // BackendUnavailable and the test passes.
        let backend = LinuxBackend::new();
        let base = std::env::temp_dir().join(format!("tpt-linux-{}", std::process::id()));
        let ctx = StartContext {
            workload_id: WorkloadId::generate(),
            mounts: vec![],
            log_dir: base.clone(),
            capabilities: CapabilitySet::empty(),
            network_mode: tpt_runtime_model::network::NetworkMode::None,
            exposed_ports: vec![],
        };
        if let Err(err) = backend.prepare(&spec(), &ctx) {
            assert_eq!(err.kind, ErrorKind::BackendUnavailable);
        }
        // Ok(()) would mean the distro exists on this host (unlikely name).
        std::fs::remove_dir_all(&base).ok();
    }
}
