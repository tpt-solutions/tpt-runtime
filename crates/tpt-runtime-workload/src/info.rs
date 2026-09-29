//! Serializable workload views for the API and CLI.

use serde::Serialize;
use std::collections::BTreeMap;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_core::state::WorkloadState;
use tpt_runtime_core::timestamp::Timestamp;
use tpt_runtime_core::ResourceUsage;
use tpt_runtime_model::BackendKind;

/// Inspectable summary of one workload (`tpt list` / `tpt inspect`).
#[derive(Clone, Debug, Serialize)]
pub struct WorkloadInfo {
    /// Workload id.
    pub id: WorkloadId,
    /// Workload name.
    pub name: String,
    /// Execution backend.
    pub backend: BackendKind,
    /// Current lifecycle state.
    pub state: WorkloadState,
    /// Creation time.
    pub created_at: Timestamp,
    /// Last start time, when started at least once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<Timestamp>,
    /// Termination time, when finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
    /// Final exit code, when finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Whether the runtime killed the workload.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub killed: bool,
    /// Granted capability names (SPEC §23: inspectable).
    pub capabilities: Vec<String>,
    /// Free-form labels from the manifest (project tagging uses
    /// `tpt.project`, see `tpt-runtime-config::project`).
    pub labels: BTreeMap<String, String>,
    /// Granted network mode.
    pub network_mode: String,
    /// Allocated inbound ports (`name → host port`).
    pub exposed_ports: BTreeMap<String, u16>,
    /// Volume mounts (`name → {mount, host path, mode}`).
    pub mounts: Vec<MountInfo>,
    /// Latest resource usage snapshot, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResourceUsage>,
    /// Backend-specific details (pid, engine, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    /// Restart count of this record.
    pub restarts: u32,
}

/// Mount summary inside [`WorkloadInfo`].
#[derive(Clone, Debug, Serialize)]
pub struct MountInfo {
    /// Volume name.
    pub name: String,
    /// Guest-visible mount point.
    pub mount: String,
    /// Host path backing the mount.
    pub host_path: String,
    /// Access mode.
    pub mode: String,
}
