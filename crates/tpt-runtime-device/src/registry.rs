//! Logical device registry (SPEC §19).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::id::DeviceId;
use tpt_runtime_model::device::DeviceAccessMode;

/// Device classes known to the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    /// Graphics/compute accelerators.
    Gpu,
    /// Cameras.
    Camera,
    /// Audio input/output.
    Audio,
    /// USB devices.
    Usb,
    /// Serial ports.
    Serial,
    /// Anything else.
    Other,
}

impl DeviceClass {
    /// Classifies a logical device id (`gpu:0` → [`DeviceClass::Gpu`]).
    pub fn of_id(id: &str) -> DeviceClass {
        match id.split(':').next().unwrap_or(id) {
            "gpu" => DeviceClass::Gpu,
            "camera" => DeviceClass::Camera,
            "audio" => DeviceClass::Audio,
            "usb" => DeviceClass::Usb,
            "serial" => DeviceClass::Serial,
            _ => DeviceClass::Other,
        }
    }
}

impl fmt::Display for DeviceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            DeviceClass::Gpu => "gpu",
            DeviceClass::Camera => "camera",
            DeviceClass::Audio => "audio",
            DeviceClass::Usb => "usb",
            DeviceClass::Serial => "serial",
            DeviceClass::Other => "other",
        };
        f.write_str(name)
    }
}

/// A device known to the runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Logical device id (`gpu:0`).
    pub id: DeviceId,
    /// Device class.
    pub class: DeviceClass,
    /// Human-readable description.
    pub description: String,
    /// Whether the device is present on this host.
    pub available: bool,
}

/// Tracks logical devices and their claims.
///
/// Discovered devices (e.g. GPUs from `tpt-runtime-gpu`) are registered as
/// available; requested-but-unknown ids fail loudly (SPEC §48).
pub struct DeviceRegistry {
    devices: BTreeMap<DeviceId, DeviceInfo>,
    claims: BTreeMap<DeviceId, Vec<String>>,
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            claims: BTreeMap::new(),
        }
    }

    /// Registers (or updates) a discovered device.
    pub fn register(&mut self, info: DeviceInfo) {
        self.devices.insert(info.id.clone(), info);
    }

    /// Lists all registered devices.
    pub fn list(&self) -> Vec<DeviceInfo> {
        self.devices.values().cloned().collect()
    }

    /// Attaches a device to a workload, enforcing availability and
    /// exclusive-class policy: every device may be claimed by multiple
    /// workloads only in `compute`/`read-only` modes.
    pub fn attach(
        &mut self,
        workload: &str,
        id: &str,
        mode: DeviceAccessMode,
    ) -> Result<DeviceInfo> {
        let key = DeviceId::from_raw(id.to_owned());
        let info = self.devices.get(&key).cloned().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("device '{id}' is not present on this host"),
            )
            .with_operation("device.attach")
        })?;
        if !info.available {
            return Err(RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("device '{id}' exists but is currently unavailable"),
            )
            .with_operation("device.attach"));
        }
        let exclusive = matches!(mode, DeviceAccessMode::Full);
        if exclusive
            && self
                .claims
                .get(&key)
                .map(|c| !c.is_empty())
                .unwrap_or(false)
        {
            return Err(RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("device '{id}' is claimed by another workload in exclusive mode"),
            )
            .with_operation("device.attach"));
        }
        self.claims
            .entry(key)
            .or_default()
            .push(workload.to_owned());
        Ok(info)
    }

    /// Detaches a workload's claim on a device.
    pub fn detach(&mut self, workload: &str, id: &str) -> Result<()> {
        let key = DeviceId::from_raw(id.to_owned());
        let claims = self.claims.get_mut(&key).ok_or_else(|| {
            RuntimeError::new(ErrorKind::NotFound, format!("device '{id}' has no claims"))
        })?;
        claims.retain(|w| w != workload);
        if claims.is_empty() {
            self.claims.remove(&key);
        }
        Ok(())
    }

    /// Drops every claim of a workload (used when it stops).
    pub fn release_workload(&mut self, workload: &str) {
        for claims in self.claims.values_mut() {
            claims.retain(|w| w != workload);
        }
        self.claims.retain(|_, v| !v.is_empty());
    }

    /// Current claim holders for a device.
    pub fn claims_of(&self, id: &str) -> Vec<String> {
        self.claims
            .get(&DeviceId::from_raw(id.to_owned()))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId::from_raw("gpu:0"),
            class: DeviceClass::Gpu,
            description: "NVIDIA GeForce RTX 3050".to_owned(),
            available: true,
        }
    }

    #[test]
    fn classifies_ids() {
        assert_eq!(DeviceClass::of_id("gpu:0"), DeviceClass::Gpu);
        assert_eq!(DeviceClass::of_id("serial:com3"), DeviceClass::Serial);
        assert_eq!(DeviceClass::of_id("mystery"), DeviceClass::Other);
    }

    #[test]
    fn attach_unknown_device_fails_loudly() {
        let mut registry = DeviceRegistry::new();
        let err = registry
            .attach("w", "gpu:9", DeviceAccessMode::Compute)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::DeviceUnavailable);
    }

    #[test]
    fn exclusive_claim_conflicts() {
        let mut registry = DeviceRegistry::new();
        registry.register(gpu());
        registry
            .attach("a", "gpu:0", DeviceAccessMode::Full)
            .unwrap();
        let err = registry
            .attach("b", "gpu:0", DeviceAccessMode::Full)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::DeviceUnavailable);
        // shared compute access is fine
        assert!(registry
            .attach("b", "gpu:0", DeviceAccessMode::Compute)
            .is_ok());
        assert_eq!(registry.claims_of("gpu:0").len(), 2);

        registry.release_workload("a");
        registry.release_workload("b");
        assert!(registry.claims_of("gpu:0").is_empty());
    }

    #[test]
    fn detach_removes_only_that_workload() {
        let mut registry = DeviceRegistry::new();
        registry.register(gpu());
        registry
            .attach("a", "gpu:0", DeviceAccessMode::Compute)
            .unwrap();
        registry
            .attach("b", "gpu:0", DeviceAccessMode::Compute)
            .unwrap();
        registry.detach("a", "gpu:0").unwrap();
        assert_eq!(registry.claims_of("gpu:0"), vec!["b".to_owned()]);
    }
}
