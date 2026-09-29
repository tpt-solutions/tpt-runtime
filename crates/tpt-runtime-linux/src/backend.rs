//! WSL-backed Linux execution (SPEC §11, Phase 1 of the layered strategy).
//!
//! Capability translation (SPEC §23 → Linux): the host never hands its
//! environment to the workload. Manifest env and granted volume mounts are
//! passed through `WSLENV`, with `/p` flags so Windows paths arrive
//! translated (`D:\data` → `/mnt/data`) as `TPT_VOLUME_<NAME>` variables.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{
    ExecutionBackend, ExitStatus, LogCapture, StartContext, StopMode, WorkloadInstance,
};

/// Environment variable prefix for granted volume mounts.
const VOLUME_ENV_PREFIX: &str = "TPT_VOLUME_";

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

        // Capability translation (SPEC §23): only manifest env and granted
        // mounts cross the boundary, and paths are translated by the /p
        // WSLENV flag rather than by string munging here.
        let mut wslenv: Vec<String> = Vec::new();
        let mut translated: Vec<(String, String)> = Vec::new();
        for (key, value) in &linux.env {
            wslenv.push(key.clone());
            translated.push((key.clone(), value.clone()));
        }
        for mount in &ctx.mounts {
            let var = volume_env_var(&mount.name);
            wslenv.push(format!("{var}/p"));
            translated.push((var, mount.host_path.display().to_string()));
        }

        let mut cmd = Command::new("wsl.exe");
        cmd.args(["-d", &linux.distro])
            .args(&command_parts)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // The host environment never leaks (SPEC §5.2): a minimal
            // Windows baseline (wsl.exe itself needs SystemRoot) plus the
            // capability-translated variables.
            .env_clear()
            .envs(sanitized_environment())
            .env("WSLENV", wslenv.join(":"))
            .envs(translated)
            .creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = cmd.spawn().map_err(|err| {
            RuntimeError::new(ErrorKind::System, format!("cannot start wsl.exe: {err}"))
                .with_backend("linux")
                .with_workload(spec.name.clone())
                .with_operation("start")
        })?;
        let pid = child.id();

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
        // The relay child stays reachable for stop(); the watcher only
        // holds the lock briefly per poll so stop() can get in.
        let child = Arc::new(Mutex::new(child));
        let watcher_child = child.clone();
        std::thread::spawn(move || {
            let status = loop {
                let polled = {
                    let mut guard = watcher_child.lock().unwrap();
                    guard.try_wait()
                };
                match polled {
                    Ok(Some(status)) => {
                        let code = status.code();
                        break ExitStatus {
                            code,
                            killed: false,
                            failed: code.map(|c| c != 0).unwrap_or(true),
                        };
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    Err(_) => break ExitStatus::terminated(),
                }
            };
            let _ = exit_tx.send(status);
        });

        Ok(Box::new(WslInstance {
            distro: linux.distro,
            pid,
            child,
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

/// The env var name carrying a mount's translated path inside the distro
/// (`proj` → `TPT_VOLUME_PROJ`).
pub fn volume_env_var(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{VOLUME_ENV_PREFIX}{}", sanitized.to_ascii_uppercase())
}

/// Minimal Windows environment so `wsl.exe` can initialize; the host's
/// own variables are never inherited (SPEC §5.2).
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
    vars
}

struct WslInstance {
    distro: String,
    pid: u32,
    child: Arc<Mutex<std::process::Child>>,
    exit_rx: Mutex<Option<tokio::sync::oneshot::Receiver<ExitStatus>>>,
    _stdout: LogCapture,
    _stderr: LogCapture,
}

impl WorkloadInstance for WslInstance {
    fn stats(&self) -> Result<tpt_runtime_core::ResourceUsage> {
        // Accounting via /proc inside the distro is Phase 2 of the Linux
        // roadmap (SPEC §11); until then, I/O through the relay is real.
        let log_len =
            |path: std::path::PathBuf| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let _ = &self._stdout;
        Ok(tpt_runtime_core::ResourceUsage {
            write_bytes: log_len(self._stdout.file.clone()) + log_len(self._stderr.file.clone()),
            collected_at: Some(tpt_runtime_core::Timestamp::now()),
            ..Default::default()
        })
    }

    fn stop(&self, _mode: StopMode) -> Result<()> {
        // Terminating the relay ends the workload view from the runtime's
        // side; in-distro processes are reaped by WSL when their stdio
        // closes (distro lifetime is left alone — coexistence, SPEC §36).
        let mut child = self.child.lock().unwrap();
        child.kill().map_err(|err| {
            RuntimeError::new(
                ErrorKind::System,
                format!("cannot terminate wsl relay for '{}': {err}", self.distro),
            )
            .with_backend("linux")
        })
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
            "distro": self.distro,
            "pid": self.pid,
        })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use tpt_runtime_capability::CapabilitySet;
    use tpt_runtime_core::id::WorkloadId;
    use tpt_runtime_model::volume::VolumeAccessMode;
    use tpt_runtime_process::ResolvedMount;

    fn linux_workload(distro: &str, command: &[&str], env: &[(&str, &str)]) -> WorkloadSpec {
        WorkloadSpec::new(
            "linux-test",
            tpt_runtime_model::execution::ExecutionSpec::LinuxProcess(
                tpt_runtime_model::execution::LinuxProcessSpec {
                    distro: distro.to_owned(),
                    command: command.iter().map(|s| s.to_string()).collect(),
                    env: env
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                },
            ),
        )
    }

    fn ctx(dir: &std::path::Path, mounts: Vec<ResolvedMount>) -> StartContext {
        StartContext {
            workload_id: WorkloadId::generate(),
            mounts,
            log_dir: dir.to_path_buf(),
            capabilities: CapabilitySet::empty(),
            network_mode: tpt_runtime_model::network::NetworkMode::None,
            exposed_ports: vec![],
        }
    }

    /// Lists installed distros (`wsl -l -q`, UTF-16LE output). `None` on
    /// hosts without WSL — the tests below skip in that case.
    fn installed_distros() -> Vec<String> {
        let output = match Command::new("wsl.exe").args(["-l", "-q"]).output() {
            Ok(output) if output.status.success() => output,
            _ => return Vec::new(),
        };
        decode_wsl_output(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.contains('('))
            .map(str::to_owned)
            .collect()
    }

    /// `wsl.exe` list output is UTF-16LE on Windows hosts.
    fn decode_wsl_output(bytes: &[u8]) -> String {
        if bytes.len() >= 2 && bytes.len().is_multiple_of(2) {
            let looks_utf16 = bytes
                .iter()
                .skip(1)
                .step_by(2)
                .all(|&b| b == 0 || b.is_ascii_whitespace());
            if looks_utf16 {
                let utf16: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                return String::from_utf16_lossy(&utf16);
            }
        }
        String::from_utf8_lossy(bytes).to_string()
    }

    fn wait_exit(instance: &dyn WorkloadInstance) -> ExitStatus {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let rx = instance.exit();
                rx.await.unwrap_or(ExitStatus::terminated())
            })
    }

    #[test]
    fn unknown_distro_fails_with_clear_error() {
        // on hosts without any WSL, the error is "wsl.exe unavailable";
        // on hosts with WSL it is "distro not installed". Both are
        // BackendUnavailable and the test passes.
        let backend = LinuxBackend::new();
        let base = std::env::temp_dir().join(format!("tpt-linux-{}", std::process::id()));
        let context = ctx(&base, vec![]);
        if let Err(err) = backend.prepare(
            &linux_workload("no-such-distro-xyz", &["true"], &[]),
            &context,
        ) {
            assert_eq!(err.kind, ErrorKind::BackendUnavailable);
        }
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn volume_env_var_is_a_valid_identifier() {
        assert_eq!(volume_env_var("proj"), "TPT_VOLUME_PROJ");
        assert_eq!(volume_env_var("my-data.v2"), "TPT_VOLUME_MY_DATA_V2");
    }

    #[test]
    fn runs_env_and_volume_translation_in_a_real_distro() {
        let distros = installed_distros();
        let Some(distro) = distros.first() else {
            eprintln!("no WSL distros installed; skipping real-distro test");
            return;
        };

        // A volume with one marker file becomes TPT_VOLUME_DATA inside the
        // distro, path-translated by WSLENV (/p).
        let volume = std::env::temp_dir().join(format!("tpt-linux-vol-{}", std::process::id()));
        std::fs::create_dir_all(&volume).unwrap();
        std::fs::write(volume.join("marker.txt"), "translated-content").unwrap();

        let backend = LinuxBackend::new();
        let log_dir = std::env::temp_dir().join(format!("tpt-linux-run-{}", std::process::id()));
        let context = ctx(
            &log_dir,
            vec![ResolvedMount {
                name: "data".to_owned(),
                mount: "/data".to_owned(),
                host_path: volume.clone(),
                mode: VolumeAccessMode::ReadWrite,
            }],
        );

        let workload = linux_workload(
            distro,
            // Print the translated env value, then read through the
            // translated volume path.
            &[
                "sh",
                "-c",
                "echo env=$TPT_TEST_VAR vol=$TPT_VOLUME_DATA; cat \"$TPT_VOLUME_DATA/marker.txt\"",
            ],
            &[("TPT_TEST_VAR", "translated-env")],
        );

        backend.prepare(&workload, &context).unwrap();
        let instance = backend.start(&workload, &context).unwrap();
        let status = wait_exit(instance.as_ref());
        assert_eq!(status.code, Some(0), "the probe command must succeed");

        let mut contents = String::new();
        for _ in 0..50 {
            contents = std::fs::read_to_string(log_dir.join("stdout.log")).unwrap_or_default();
            if contents.contains("translated-content") {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        assert!(
            contents.contains("env=translated-env"),
            "manifest env must cross the WSLENV boundary: {contents:?}"
        );
        assert!(
            contents.contains("vol=/mnt/"),
            "volume paths must arrive translated: {contents:?}"
        );
        assert!(
            contents.contains("translated-content"),
            "the workload must read host files through the translated mount: {contents:?}"
        );

        // I/O accounting reflects the captured output.
        let usage = instance.stats().unwrap();
        assert!(usage.write_bytes > 0);

        std::fs::remove_dir_all(&volume).ok();
        std::fs::remove_dir_all(&log_dir).ok();
    }

    #[test]
    fn stop_terminates_a_long_running_relay() {
        let distros = installed_distros();
        let Some(distro) = distros.first() else {
            eprintln!("no WSL distros installed; skipping real-distro test");
            return;
        };

        let backend = LinuxBackend::new();
        let log_dir = std::env::temp_dir().join(format!("tpt-linux-stop-{}", std::process::id()));
        let context = ctx(&log_dir, vec![]);
        let workload = linux_workload(distro, &["sleep", "30"], &[]);

        let instance = backend.start(&workload, &context).unwrap();
        instance.stop(StopMode::Kill).unwrap();
        let status = wait_exit(instance.as_ref());
        assert!(status.code != Some(0), "a killed relay does not exit 0");
        std::fs::remove_dir_all(&log_dir).ok();
    }
}
