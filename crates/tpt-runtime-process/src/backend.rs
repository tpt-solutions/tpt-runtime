//! Backend trait and workload instances (SPEC §10).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tpt_runtime_core::error::Result;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_core::ResourceUsage;
use tpt_runtime_model::volume::VolumeAccessMode;
use tpt_runtime_model::workload::WorkloadSpec;

/// How a stop request should be delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    /// Ask politely; backends without a graceful mechanism kill.
    Graceful,
    /// Terminate immediately.
    Kill,
}

/// Terminal status of a workload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// Process/module exit code; `None` when killed or unknown.
    pub code: Option<i32>,
    /// Whether the runtime terminated the workload.
    pub killed: bool,
    /// Whether the workload reports a failure (non-zero exit / trap).
    pub failed: bool,
}

impl ExitStatus {
    /// A clean exit with code 0.
    pub fn success() -> Self {
        Self {
            code: Some(0),
            killed: false,
            failed: false,
        }
    }

    /// Killed by the runtime.
    pub fn terminated() -> Self {
        Self {
            code: None,
            killed: true,
            failed: true,
        }
    }
}

/// A volume mount resolved to its host location for a specific workload
/// instance (SPEC §15: one volume, many views).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMount {
    /// Logical volume name.
    pub name: String,
    /// Mount point as requested by the manifest.
    pub mount: String,
    /// Host path backing the view for this workload.
    pub host_path: PathBuf,
    /// Access mode of this view.
    pub mode: VolumeAccessMode,
}

/// Everything a backend needs to prepare and start one workload.
#[derive(Clone, Debug)]
pub struct StartContext {
    /// Workload id assigned by the manager.
    pub workload_id: WorkloadId,
    /// Resolved volume mounts.
    pub mounts: Vec<ResolvedMount>,
    /// Directory for this workload's logs.
    pub log_dir: PathBuf,
    /// Capabilities granted to this workload (SPEC §23).
    pub capabilities: tpt_runtime_capability::CapabilitySet,
    /// Network mode granted (SPEC §18).
    pub network_mode: tpt_runtime_model::network::NetworkMode,
    /// Host ports exposed for this workload.
    pub exposed_ports: Vec<(String, u16)>,
}

/// The interface every execution backend implements (SPEC §10).
///
/// Methods are synchronous: backends do their own threading and hand back
/// [`WorkloadInstance`] handles. The manager invokes them off the async
/// runtime's core threads.
pub trait ExecutionBackend: Send + Sync {
    /// Which backend kind this implements.
    fn kind(&self) -> tpt_runtime_model::BackendKind;

    /// Validates and prepares execution resources without starting
    /// (`defined → resolved → prepared`, SPEC §3.1). Must be idempotent.
    fn prepare(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<()>;

    /// Starts the workload and returns a live instance handle
    /// (`created → starting → running`).
    fn start(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>>;
}

/// A live, started workload.
///
/// The manager polls [`WorkloadInstance::stats`] for observability, awaits
/// [`WorkloadInstance::exit`] for termination, and calls
/// [`WorkloadInstance::stop`] on lifecycle stop/kill operations.
pub trait WorkloadInstance: Send + Sync {
    /// Snapshot of actual resource usage (SPEC §25, §27).
    fn stats(&self) -> Result<ResourceUsage>;

    /// Stops the workload: `Graceful` asks, `Kill` terminates. Backends
    /// without graceful shutdown semantics (e.g. native Windows processes)
    /// treat both as termination and document it.
    fn stop(&self, mode: StopMode) -> Result<()>;

    /// Resolves when the workload exits; the receiver gets the final
    /// [`ExitStatus`]. The receiver must complete even if the workload is
    /// killed or crashes.
    fn exit(&self) -> tokio::sync::oneshot::Receiver<ExitStatus>;

    /// Backend-specific details for `tpt inspect` (pid, engine, ...).
    fn describe(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

