//! The [`SandboxProvider`] seam and its Windows implementation.
//!
//! [`SandboxProvider`] is the boundary `tpt-runtime-oci` was always waiting
//! for: given a prepared [`Bundle`], start an isolated workload. Keeping it a
//! trait means Boxcar (or any other sandbox) can replace [`WindowsSandbox`]
//! without `tpt-runtime-oci` changing.

use std::sync::{Arc, Mutex};

use tpt_runtime_core::error::Result;
use tpt_runtime_core::ResourceUsage;
use tpt_runtime_oci::Bundle;
use tpt_runtime_process::{ExitStatus, StartContext, StopMode, WorkloadInstance};

use crate::bundle_env;
use crate::mounts;
use crate::network;

/// Resource ceilings the provider applies to each workload (SPEC §25).
///
/// These live on the provider rather than on [`StartContext`] because they
/// are sandbox parameters, not workload-model state: the same limits apply to
/// every workload, and keeping them here means `tpt-runtime-process` does not
/// have to grow an OCI-specific field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SandboxLimits {
    /// Per-process memory limit in bytes; `None` leaves it to the job default.
    pub memory_bytes: Option<u64>,
    /// Cap on concurrent processes in the job; `None` uses the job default.
    pub max_processes: Option<u32>,
}

impl SandboxLimits {
    /// Limits derived from a workload's resource request.
    pub fn from_resources(resources: &tpt_runtime_model::resources::ResourceSpec) -> Self {
        Self {
            memory_bytes: resources.memory.map(|memory| memory.0),
            max_processes: None,
        }
    }
}

/// The isolation boundary a prepared bundle is handed to.
///
/// `tpt-boxcar` was the intended provider, but its Origin sandbox does not
/// spawn containers (`type: oci` is documented as bookkeeping-only) and has
/// no Windows isolation, so the runtime ships its own
/// (`tpt-runtime-sandbox`). The trait keeps either one swappable without
/// `tpt-runtime-oci` depending on either.
pub trait SandboxProvider: Send + Sync {
    /// Starts the bundle as an isolated workload.
    ///
    /// `limits` carries the resource ceilings already resolved from the
    /// manifest by the caller, so a provider never has to re-read the spec.
    fn start_bundle(
        &self,
        bundle: &Bundle,
        ctx: &StartContext,
        limits: SandboxLimits,
    ) -> Result<Box<dyn WorkloadInstance>>;
}

/// The runtime's own Windows isolation provider.
///
/// Construct with [`WindowsSandbox::new`]. All isolation happens in the
/// platform layer (`job`, `token`, `win32`); this type is the policy and
/// lifecycle glue.
#[derive(Debug, Default)]
pub struct WindowsSandbox;

impl WindowsSandbox {
    /// Creates the provider.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
impl SandboxProvider for WindowsSandbox {
    fn start_bundle(
        &self,
        bundle: &Bundle,
        ctx: &StartContext,
        limits: SandboxLimits,
    ) -> Result<Box<dyn WorkloadInstance>> {
        use crate::job::JobObject;
        use crate::win32::spawn_suspended;

        let plan = bundle_env::plan(bundle)?;
        // Validate the granted mounts and network intent *before* the log dir
        // is created or any process is spawned, so a misconfigured workload
        // fails without leaving anything behind.
        mounts::validate_mounts(ctx)?;
        network::validate_network(ctx)?;

        // The log dir must exist before the child writes to the pipes.
        std::fs::create_dir_all(&ctx.log_dir)?;

        let job = JobObject::with_limits(limits.memory_bytes, limits.max_processes)?;

        // Image and manifest environment first, then the runtime's own
        // translations (mounts, network mode) so a manifest cannot shadow a
        // runtime-owned variable by naming it in the image.
        let mut env = plan.env.clone();
        for (key, value) in mounts::mount_environment(ctx) {
            env.retain(|(existing, _)| existing != &key);
            env.push((key, value));
        }
        for (key, value) in network::network_environment(ctx) {
            env.retain(|(existing, _)| existing != &key);
            env.push((key, value));
        }
        let env_block = bundle_env::environment_block(&env);

        let started = spawn_suspended(
            &plan.program,
            &plan.args,
            &env_block,
            &plan.working_dir,
            &job,
        )
        .map_err(|err| {
            err.with_backend("sandbox")
                .with_workload(ctx.workload_id.to_string())
        })?;

        let stdout = tpt_runtime_process::LogCapture::create(&ctx.log_dir, "stdout")?;
        let stderr = tpt_runtime_process::LogCapture::create(&ctx.log_dir, "stderr")?;
        stdout.spawn_reader(started.stdout);
        stderr.spawn_reader(started.stderr);

        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<ExitStatus>();
        let terminate_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // The watcher owns the process handles and the job: it reaps the child
        // and only then drops the job, so kill-on-close never races the exit.
        let watcher_handles = started.handles;
        let watcher_job = job.clone();
        let watcher_flag = terminate_requested.clone();
        std::thread::spawn(move || {
            let status = match watcher_handles.wait() {
                Ok(code) => {
                    let killed = watcher_flag.load(std::sync::atomic::Ordering::SeqCst);
                    ExitStatus {
                        code: Some(code as i32),
                        killed,
                        // a runtime-requested termination is not a crash
                        failed: !killed && code != 0,
                    }
                }
                Err(_) => ExitStatus::terminated(),
            };
            let _ = exit_tx.send(status);
            drop(watcher_job);
        });

        Ok(Box::new(SandboxInstance {
            pid: started.pid,
            program: plan.program,
            bundle_id: bundle.id.clone(),
            mounts: mounts::describe_mounts(ctx),
            network: network::describe_network(ctx),
            _job: job,
            terminate_requested,
            exit_rx: Mutex::new(Some(exit_rx)),
            _stdout: stdout,
            _stderr: stderr,
        }))
    }
}

#[cfg(not(windows))]
impl SandboxProvider for WindowsSandbox {
    fn start_bundle(
        &self,
        _bundle: &Bundle,
        _ctx: &StartContext,
        _limits: SandboxLimits,
    ) -> Result<Box<dyn WorkloadInstance>> {
        Err(tpt_runtime_core::error::RuntimeError::new(
            tpt_runtime_core::error::ErrorKind::BackendUnavailable,
            "the Windows sandbox provider requires a Windows host",
        )
        .with_backend("sandbox"))
    }
}

/// `tpt-runtime-oci`'s isolation seam, implemented on Windows.
///
/// The provider builds its own [`SandboxLimits`] from the manifest's
/// resource request, so the OCI backend never has to know how the sandbox
/// enforces them.
#[cfg(windows)]
impl tpt_runtime_oci::IsolationProvider for WindowsSandbox {
    fn start(
        &self,
        bundle: &Bundle,
        spec: &tpt_runtime_model::workload::WorkloadSpec,
        ctx: &StartContext,
    ) -> Result<Box<dyn WorkloadInstance>> {
        let limits = SandboxLimits::from_resources(&spec.resources);
        self.start_bundle(bundle, ctx, limits)
    }
}

#[cfg(not(windows))]
impl tpt_runtime_oci::IsolationProvider for WindowsSandbox {
    fn start(
        &self,
        _bundle: &Bundle,
        _spec: &tpt_runtime_model::workload::WorkloadSpec,
        _ctx: &StartContext,
    ) -> Result<Box<dyn WorkloadInstance>> {
        Err(tpt_runtime_core::error::RuntimeError::new(
            tpt_runtime_core::error::ErrorKind::BackendUnavailable,
            "the Windows sandbox provider requires a Windows host",
        )
        .with_backend("sandbox"))
    }
}

/// Live handle for one sandboxed OCI workload.
#[cfg(windows)]
struct SandboxInstance {
    pid: u32,
    program: std::path::PathBuf,
    bundle_id: String,
    /// The mounts and network posture, captured at start so `inspect` reports
    /// what this workload was actually granted.
    mounts: Vec<serde_json::Value>,
    network: serde_json::Value,
    /// Keeps the job (and its kill-on-close guarantee) alive for the
    /// workload's whole lifetime.
    _job: Arc<crate::job::JobObject>,
    terminate_requested: Arc<std::sync::atomic::AtomicBool>,
    exit_rx: Mutex<Option<tokio::sync::oneshot::Receiver<ExitStatus>>>,
    _stdout: tpt_runtime_process::LogCapture,
    _stderr: tpt_runtime_process::LogCapture,
}

#[cfg(windows)]
impl WorkloadInstance for SandboxInstance {
    fn stats(&self) -> Result<ResourceUsage> {
        let mut usage = ResourceUsage::default();
        if let Some(job_stats) = self._job.query_usage() {
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
        // Windows workloads have no universally honored graceful signal; job
        // termination is the documented stop path, exactly as for native
        // processes (SPEC §12).
        self.terminate_requested
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self._job.terminate(1)
    }

    fn exit(&self) -> tokio::sync::oneshot::Receiver<ExitStatus> {
        let mut guard = self.exit_rx.lock().unwrap();
        match guard.take() {
            Some(rx) => rx,
            None => {
                // Second exit request: the first receiver already holds the
                // status. Terminate and resolve immediately so the trait
                // contract (the receiver always completes) holds.
                let _ = self._job.terminate(1);
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = tx.send(ExitStatus::terminated());
                rx
            }
        }
    }

    fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": "sandbox",
            "provider": "windows-job-token",
            "pid": self.pid,
            "bundle": self.bundle_id,
            "program": self.program.display().to_string(),
            "isolation": {
                "job_object": true,
                "restricted_token": true,
                "handle_inheritance_list": true,
            },
            "mounts": self.mounts,
            "network": self.network,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_come_from_the_resource_request() {
        let resources = tpt_runtime_model::resources::ResourceSpec {
            memory: Some(tpt_runtime_model::resources::Memory::gib(2)),
            ..Default::default()
        };
        let limits = SandboxLimits::from_resources(&resources);
        assert_eq!(limits.memory_bytes, Some(2 * 1024 * 1024 * 1024));
        assert_eq!(limits.max_processes, None);
    }

    #[test]
    fn default_limits_impose_no_memory_cap() {
        let limits =
            SandboxLimits::from_resources(&tpt_runtime_model::resources::ResourceSpec::default());
        assert_eq!(limits, SandboxLimits::default());
    }
}
