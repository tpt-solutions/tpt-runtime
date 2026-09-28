//! The workload specification: the runtime's primary abstraction (SPEC §5.1, §9).

use crate::backend::BackendKind;
use crate::device::DeviceRequest;
use crate::execution::ExecutionSpec;
use crate::network::NetworkSpec;
use crate::resources::ResourceSpec;
use crate::volume::VolumeMount;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// A complete, declarative workload definition (SPEC §9).
///
/// A `WorkloadSpec` says what a workload requires and what it is allowed to
/// access; it never says how a backend must realize those requirements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Human-readable workload name (unique among running workloads).
    pub name: String,
    /// Backend-independent execution definition.
    pub execution: ExecutionSpec,
    /// Resource requests (SPEC §25).
    #[serde(default)]
    pub resources: ResourceSpec,
    /// Logical volume mounts (SPEC §15).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<VolumeMount>,
    /// Logical network request (SPEC §18).
    #[serde(default)]
    pub network: NetworkSpec,
    /// Device requests (SPEC §19).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<DeviceRequest>,
    /// Requested capabilities by name (SPEC §23), e.g. `network.outbound`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Free-form labels for selection and auditing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

impl WorkloadSpec {
    /// Creates a workload spec with default resources and no grants;
    /// fill in [`resources`](WorkloadSpec::resources),
    /// [`network`](WorkloadSpec::network) and friends afterwards.
    pub fn new(name: impl Into<String>, execution: ExecutionSpec) -> Self {
        Self {
            name: name.into(),
            execution,
            resources: ResourceSpec::default(),
            volumes: Vec::new(),
            network: NetworkSpec::default(),
            devices: Vec::new(),
            capabilities: Vec::new(),
            labels: BTreeMap::new(),
        }
    }

    /// The backend that will execute this workload.
    pub fn backend(&self) -> BackendKind {
        self.execution.backend()
    }

    /// Validates the whole specification, returning the first error found.
    pub fn validate(&self) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "workload name must not be empty",
            ));
        }
        if name.len() > 128 || !name.chars().all(is_name_char) {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("workload name '{name}' must be 1..=128 characters of [A-Za-z0-9._-]"),
            ));
        }
        self.execution.validate()?;
        self.resources.validate()?;
        for mount in &self.volumes {
            if mount.name.trim().is_empty() || mount.mount.trim().is_empty() {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    "volume mounts require both a volume name and a mount point",
                ));
            }
        }
        if !self.network.expose.is_empty() && !self.network.mode.allows_inbound() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!(
                    "network mode '{}' cannot expose ports (requires host or service)",
                    self.network.mode
                ),
            ));
        }
        Ok(())
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{WindowsProcessSpec};

    fn sample() -> WorkloadSpec {
        WorkloadSpec {
            name: "build-agent".to_owned(),
            execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
                program: "myapp.exe".to_owned(),
                ..Default::default()
            }),
            resources: ResourceSpec {
                cpu: Some(4.0),
                memory: Some(crate::resources::Memory::gib(4)),
                ..Default::default()
            },
            network: NetworkSpec {
                mode: crate::network::NetworkMode::Outbound,
                ..Default::default()
            },
            volumes: vec![VolumeMount {
                name: "project".to_owned(),
                mount: "/workspace".to_owned(),
                mode: crate::volume::VolumeAccessMode::ReadWrite,
            }],
            devices: vec![],
            capabilities: vec!["network.outbound".to_owned()],
            labels: BTreeMap::new(),
        }
    }

    #[test]
    fn accepts_valid_spec() {
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn rejects_empty_name_and_bad_names() {
        let mut spec = sample();
        spec.name = "  ".to_owned();
        assert!(spec.validate().is_err());
        spec.name = "bad name!".to_owned();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn rejects_exposed_ports_without_inbound_mode() {
        let mut spec = sample();
        spec.network.mode = crate::network::NetworkMode::Outbound;
        spec.network.expose.insert("http".to_owned(), 8080);
        assert!(spec.validate().is_err());
    }

    #[test]
    fn json_round_trip_preserves_spec() {
        let spec = sample();
        let json = serde_json::to_string(&spec).unwrap();
        let back: WorkloadSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
        assert_eq!(back.backend(), BackendKind::Windows);
    }
}
