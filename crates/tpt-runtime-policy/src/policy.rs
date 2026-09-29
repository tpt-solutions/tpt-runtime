//! Resource policy evaluation (SPEC §25) and admission decisions.

use serde::{Deserialize, Serialize};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::resources::Memory;
use tpt_runtime_model::workload::WorkloadSpec;

/// How a resource limit may be enforced (SPEC §25).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePolicy {
    /// Request is denied when the limit cannot be satisfied.
    #[default]
    HardLimit,
    /// Request is admitted and clamped to what is available.
    SoftLimit,
    /// Capacity is reserved up front; failure to reserve is fatal.
    Reservation,
    /// Higher priority workloads reclaim capacity first (advisory in MVP).
    Priority,
    /// Admitted whenever any capacity remains.
    BestEffort,
}

/// Resource classes tracked by the runtime (SPEC §25).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceClass {
    Cpu,
    Memory,
    Storage,
    Network,
    Gpu,
    ProcessCount,
}

/// Host capacity known to the policy engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostCapacity {
    /// Total CPU cores available for workloads.
    pub cpu_cores: f64,
    /// Total memory available for workloads.
    pub memory: Memory,
    /// Number of GPUs discovered, if any.
    pub gpus: u32,
}

impl HostCapacity {
    /// Conservative defaults used when discovery is unavailable.
    pub fn unknown() -> Self {
        Self {
            cpu_cores: f64::INFINITY,
            memory: Memory(u64::MAX),
            gpus: 0,
        }
    }
}

/// The result of admitting one workload.
#[derive(Clone, Debug, PartialEq)]
pub enum PolicyDecision {
    /// The request is admitted as asked.
    Admitted,
    /// The request is admitted with adjustments; carries explanations.
    Adjusted(Vec<String>),
    /// The request is denied; carries the reason.
    Denied(String),
}

impl PolicyDecision {
    /// Whether the workload may run after this decision.
    pub fn is_admitted(&self) -> bool {
        !matches!(self, PolicyDecision::Denied(_))
    }
}

/// Pure admission engine: requests in, decisions out (SPEC §25: expose both
/// requested and actual usage; explicit failure over silent degradation,
/// SPEC §48).
#[derive(Clone, Debug)]
pub struct PolicyEngine {
    capacity: HostCapacity,
    defaults: ResourcePolicy,
}

impl PolicyEngine {
    /// Creates an engine with explicit capacity knowledge.
    pub fn new(capacity: HostCapacity) -> Self {
        Self {
            capacity,
            defaults: ResourcePolicy::HardLimit,
        }
    }

    /// Overrides the default policy applied when a request is unsatisfiable.
    pub fn with_default_policy(mut self, policy: ResourcePolicy) -> Self {
        self.defaults = policy;
        self
    }

    /// Admits a workload specification against host capacity.
    ///
    /// Hard limits deny unsatisfiable requests; soft limits clamp; best
    /// effort admits. `Reservation` behaves like a hard limit in the MVP
    /// because capacity is not yet reclaimable.
    pub fn admit(&self, spec: &WorkloadSpec) -> Result<PolicyDecision> {
        let mut notes = Vec::new();

        if let Some(cpu) = spec.resources.cpu {
            if cpu > self.capacity.cpu_cores {
                match self.defaults {
                    ResourcePolicy::HardLimit | ResourcePolicy::Reservation => {
                        return Ok(PolicyDecision::Denied(format!(
                            "cpu request {cpu} exceeds available capacity {:.1}",
                            self.capacity.cpu_cores
                        )));
                    }
                    ResourcePolicy::SoftLimit => {
                        notes.push(format!("cpu clamped to {:.1}", self.capacity.cpu_cores));
                    }
                    ResourcePolicy::Priority | ResourcePolicy::BestEffort => {}
                }
            }
        }

        if let Some(memory) = spec.resources.memory {
            if memory > self.capacity.memory {
                match self.defaults {
                    ResourcePolicy::HardLimit | ResourcePolicy::Reservation => {
                        return Ok(PolicyDecision::Denied(format!(
                            "memory request {} exceeds available capacity {}",
                            memory, self.capacity.memory
                        )));
                    }
                    ResourcePolicy::SoftLimit => {
                        notes.push(format!("memory clamped to {}", self.capacity.memory));
                    }
                    ResourcePolicy::Priority | ResourcePolicy::BestEffort => {}
                }
            }
        }

        // GPU is always a capability decision: requesting a GPU device that
        // does not exist must fail loudly (SPEC §19, §48).
        for device in &spec.devices {
            if device.id.starts_with("gpu") {
                let requested_index = device.id.rsplit(':').next().unwrap_or("");
                let available = self.capacity.gpus;
                let index_ok = requested_index
                    .parse::<u32>()
                    .map(|i| i < available)
                    .unwrap_or(available > 0);
                if !index_ok {
                    return Ok(PolicyDecision::Denied(format!(
                        "device '{}' requested but no matching gpu is present ({} discovered)",
                        device.id, available
                    )));
                }
            }
        }

        if notes.is_empty() {
            Ok(PolicyDecision::Admitted)
        } else {
            Ok(PolicyDecision::Adjusted(notes))
        }
    }

    /// Convenience for the daemon: admits or fails with `ResourceExhausted`.
    pub fn enforce(&self, spec: &WorkloadSpec) -> Result<()> {
        match self.admit(spec)? {
            PolicyDecision::Admitted => Ok(()),
            PolicyDecision::Adjusted(notes) => {
                // Adjustments are surfaced, not fatal; callers log them.
                let _ = notes;
                Ok(())
            }
            PolicyDecision::Denied(reason) => {
                Err(RuntimeError::new(ErrorKind::ResourceExhausted, reason)
                    .with_workload(spec.name.clone())
                    .with_operation("admit"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_runtime_model::device::{DeviceAccessMode, DeviceRequest};
    use tpt_runtime_model::execution::{ExecutionSpec, WindowsProcessSpec};

    fn spec(cpu: Option<f64>, memory: Option<Memory>, devices: Vec<DeviceRequest>) -> WorkloadSpec {
        WorkloadSpec {
            name: "test".to_owned(),
            execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
                program: "a.exe".to_owned(),
                ..Default::default()
            }),
            resources: tpt_runtime_model::resources::ResourceSpec {
                cpu,
                memory,
                ..Default::default()
            },
            volumes: vec![],
            network: Default::default(),
            devices,
            capabilities: vec![],
            labels: Default::default(),
        }
    }

    fn engine() -> PolicyEngine {
        PolicyEngine::new(HostCapacity {
            cpu_cores: 8.0,
            memory: Memory::gib(16),
            gpus: 1,
        })
    }

    #[test]
    fn admits_within_capacity() {
        let decision = engine()
            .admit(&spec(Some(4.0), Some(Memory::gib(8)), vec![]))
            .unwrap();
        assert_eq!(decision, PolicyDecision::Admitted);
    }

    #[test]
    fn hard_limit_denies_overcommit() {
        let decision = engine()
            .admit(&spec(Some(32.0), Some(Memory::gib(64)), vec![]))
            .unwrap();
        assert!(matches!(decision, PolicyDecision::Denied(_)));
        engine()
            .enforce(&spec(Some(32.0), None, vec![]))
            .unwrap_err();
    }

    #[test]
    fn soft_limit_clamps() {
        let engine = engine().with_default_policy(ResourcePolicy::SoftLimit);
        let decision = engine.admit(&spec(Some(32.0), None, vec![])).unwrap();
        match decision {
            PolicyDecision::Adjusted(notes) => assert!(notes[0].contains("cpu clamped")),
            other => panic!("expected adjustment, got {other:?}"),
        }
    }

    #[test]
    fn missing_gpu_denies_loudly() {
        let decision = engine()
            .admit(&spec(
                None,
                None,
                vec![DeviceRequest {
                    id: "gpu:3".to_owned(),
                    mode: DeviceAccessMode::Compute,
                }],
            ))
            .unwrap();
        assert!(matches!(decision, PolicyDecision::Denied(reason) if reason.contains("gpu:3")));
    }

    #[test]
    fn existing_gpu_admitted() {
        let decision = engine()
            .admit(&spec(
                None,
                None,
                vec![DeviceRequest {
                    id: "gpu:0".to_owned(),
                    mode: DeviceAccessMode::Compute,
                }],
            ))
            .unwrap();
        assert!(decision.is_admitted());
    }
}
