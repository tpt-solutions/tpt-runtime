//! Per-workload usage snapshots (SPEC §25, §27).

use std::collections::BTreeMap;
use tpt_runtime_core::error::Result;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_core::ResourceUsage;

/// Keeps the latest usage snapshot per workload. Backends push; the API
/// serves; nothing polls aggressively (SPEC §47: minimal telemetry
/// overhead).
#[derive(Default)]
pub struct MetricsRegistry {
    usage: std::sync::Mutex<BTreeMap<WorkloadId, ResourceUsage>>,
}

impl MetricsRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the latest snapshot for a workload.
    pub fn update(&self, id: &WorkloadId, usage: ResourceUsage) {
        self.usage.lock().unwrap().insert(id.clone(), usage);
    }

    /// Latest snapshot for a workload.
    pub fn get(&self, id: &WorkloadId) -> Option<ResourceUsage> {
        self.usage.lock().unwrap().get(id).cloned()
    }

    /// All snapshots (for `tpt list` / `tpt status`).
    pub fn all(&self) -> Vec<(WorkloadId, ResourceUsage)> {
        self.usage
            .lock()
            .unwrap()
            .iter()
            .map(|(id, usage)| (id.clone(), usage.clone()))
            .collect()
    }

    /// Forgets a workload's metrics when it is destroyed.
    pub fn remove(&self, id: &WorkloadId) {
        self.usage.lock().unwrap().remove(id);
    }
}

impl MetricsRegistry {
    /// Collects usage from an instance and records it.
    pub fn collect_from(&self, id: &WorkloadId, sample: impl FnOnce() -> Result<ResourceUsage>) {
        if let Ok(usage) = sample() {
            self.update(id, usage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_get_remove_round_trip() {
        let registry = MetricsRegistry::new();
        let id = WorkloadId::generate();
        assert!(registry.get(&id).is_none());

        registry.update(
            &id,
            ResourceUsage {
                memory_peak_bytes: 4096,
                ..Default::default()
            },
        );
        assert_eq!(registry.get(&id).unwrap().memory_peak_bytes, 4096);
        assert_eq!(registry.all().len(), 1);

        registry.remove(&id);
        assert!(registry.get(&id).is_none());
    }

    #[test]
    fn collect_from_ignores_errors() {
        let registry = MetricsRegistry::new();
        let id = WorkloadId::generate();
        registry.collect_from(&id, || {
            Err(tpt_runtime_core::RuntimeError::new(
                tpt_runtime_core::error::ErrorKind::System,
                "job query failed",
            ))
        });
        assert!(registry.get(&id).is_none());
    }
}
