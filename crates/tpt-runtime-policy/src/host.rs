//! Host capacity discovery (SPEC §25): real CPU/memory limits for
//! admission, instead of assuming infinity. GPU count comes from the
//! caller (the daemon's discovery pass); GPU *memory* totals arrive with
//! the NVIDIA integration.

use crate::HostCapacity;
use tpt_runtime_model::resources::Memory;

impl HostCapacity {
    /// Discovers this host's capacity: CPU cores from the scheduler, RAM
    /// from the OS, GPU count as given. Any field that cannot be read
    /// falls back to "unbounded" so discovery problems degrade admission,
    /// never the runtime.
    pub fn discover(gpus: u32) -> Self {
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get() as f64)
            .unwrap_or(f64::INFINITY);
        Self {
            cpu_cores,
            memory: discover_memory().unwrap_or(Memory(u64::MAX)),
            gpus,
        }
    }
}

#[cfg(windows)]
fn discover_memory() -> Option<Memory> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: `status` is a zeroed, correctly-sized MEMORYSTATUSEX as the
    // API requires (dwLength is set before the call).
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: valid pointer to the initialized struct.
    if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
        Some(Memory(status.ullTotalPhys))
    } else {
        None
    }
}

#[cfg(not(windows))]
fn discover_memory() -> Option<Memory> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(Memory(kb * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_returns_sane_host_limits() {
        let capacity = HostCapacity::discover(2);
        assert!(capacity.cpu_cores >= 1.0, "cpu: {}", capacity.cpu_cores);
        assert!(
            capacity.memory.0 > Memory::gib(1).0,
            "memory: {}",
            capacity.memory
        );
        assert_eq!(capacity.gpus, 2);
    }

    #[test]
    fn discovered_capacity_denies_absurd_requests() {
        use crate::PolicyEngine;
        use tpt_runtime_model::device::{DeviceAccessMode, DeviceRequest};
        use tpt_runtime_model::execution::{ExecutionSpec, WindowsProcessSpec};
        use tpt_runtime_model::workload::WorkloadSpec;

        let engine = PolicyEngine::new(HostCapacity::discover(0));
        let spec = WorkloadSpec {
            name: "hog".to_owned(),
            execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
                program: "a.exe".to_owned(),
                ..Default::default()
            }),
            resources: tpt_runtime_model::resources::ResourceSpec {
                memory: Some(Memory::gib(1 << 20)), // 1 TiB
                ..Default::default()
            },
            volumes: vec![],
            network: Default::default(),
            devices: vec![DeviceRequest {
                id: "gpu:0".to_owned(),
                mode: DeviceAccessMode::Compute,
            }],
            capabilities: vec![],
            labels: Default::default(),
        };
        let decision = engine.admit(&spec).unwrap();
        assert!(
            !decision.is_admitted(),
            "a 1 TiB request and an undiscovered GPU must be denied"
        );
    }
}
