//! GPU discovery via `nvidia-smi` (SPEC §20).

use serde::{Deserialize, Serialize};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::id::DeviceId;
use tpt_runtime_device::{DeviceClass, DeviceInfo};

/// One discovered GPU.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuInfo {
    /// Zero-based device index.
    pub index: u32,
    /// Marketing name.
    pub name: String,
    /// Total memory in MiB (as reported by the driver).
    pub memory_total_mib: u64,
    /// Driver version string.
    pub driver_version: String,
    /// Stable UUID (`GPU-xxxx`), when reported.
    pub uuid: Option<String>,
}

impl GpuInfo {
    /// The logical device id for this GPU (`gpu:<index>`).
    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_raw(format!("gpu:{}", self.index))
    }

    /// Converts the GPU into a registry device entry.
    pub fn as_device(&self) -> DeviceInfo {
        DeviceInfo {
            id: self.device_id(),
            class: DeviceClass::Gpu,
            description: format!("{} ({} MiB)", self.name, self.memory_total_mib),
            available: true,
        }
    }
}

/// Discovers NVIDIA GPUs via `nvidia-smi`.
///
/// Returns an empty list when `nvidia-smi` is absent (no NVIDIA stack) or
/// unreadable; a present-but-broken driver surfaces as an error so hosts
/// can distinguish "no gpu" from "broken gpu" (SPEC §48).
pub fn discover_gpus() -> Result<Vec<GpuInfo>> {
    let output = match std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.total,driver_version,uuid",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        // 0 commonly means "no devices" for this query
        Ok(output) if output.status.code() == Some(0) => output,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not supported") || stderr.contains("No devices") {
                return Ok(Vec::new());
            }
            return Err(RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("nvidia-smi failed: {}", stderr.trim()),
            ));
        }
        Err(_) => return Ok(Vec::new()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut gpus = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(", ").map(str::trim).collect();
        if fields.len() < 5 {
            continue;
        }
        let index: u32 = fields[0].parse().map_err(|_| {
            RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("unparsable nvidia-smi output: '{line}'"),
            )
        })?;
        gpus.push(GpuInfo {
            index,
            name: fields[1].to_owned(),
            memory_total_mib: fields[2].parse().unwrap_or(0),
            driver_version: fields[3].to_owned(),
            uuid: Some(fields[4].to_owned()).filter(|u| u != "[N/A]"),
        });
    }
    Ok(gpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_is_safe_on_any_host() {
        // must not fail on hosts without nvidia-smi
        let gpus = discover_gpus().unwrap();
        // on a host WITH nvidia-smi the entries must be well-formed
        for gpu in &gpus {
            assert_eq!(gpu.device_id().as_str(), format!("gpu:{}", gpu.index));
            assert!(!gpu.name.is_empty());
        }
    }

    #[test]
    fn device_registration_shape() {
        let gpu = GpuInfo {
            index: 0,
            name: "Test GPU".to_owned(),
            memory_total_mib: 8192,
            driver_version: "550.00".to_owned(),
            uuid: Some("GPU-test".to_owned()),
        };
        let device = gpu.as_device();
        assert_eq!(device.class, DeviceClass::Gpu);
        assert!(device.available);
        assert!(device.description.contains("8192"));
    }
}
