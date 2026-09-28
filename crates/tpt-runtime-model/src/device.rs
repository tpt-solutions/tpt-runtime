//! Device requests (SPEC §19).
//!
//! Devices are capabilities: a workload requests access by logical id
//! (`gpu:0`, `camera:0`, `serial:com3`), and the runtime decides how the
//! access is implemented on the host.

use serde::{Deserialize, Serialize};
use std::fmt;

/// How a workload may use a device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAccessMode {
    /// Compute access only (e.g. CUDA without display).
    #[default]
    Compute,
    /// Full access including display paths.
    Full,
    /// Read-only access.
    ReadOnly,
}

impl std::str::FromStr for DeviceAccessMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "compute" => Ok(DeviceAccessMode::Compute),
            "full" => Ok(DeviceAccessMode::Full),
            "read-only" | "readonly" | "ro" => Ok(DeviceAccessMode::ReadOnly),
            other => Err(format!("unknown device mode '{other}'")),
        }
    }
}

impl fmt::Display for DeviceAccessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceAccessMode::Compute => f.write_str("compute"),
            DeviceAccessMode::Full => f.write_str("full"),
            DeviceAccessMode::ReadOnly => f.write_str("read-only"),
        }
    }
}

/// A device access request (SPEC §19 `[[devices]]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRequest {
    /// Logical device id such as `gpu:0` or `audio:0`.
    pub id: String,
    /// Requested access mode.
    #[serde(default)]
    pub mode: DeviceAccessMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_request_defaults_to_compute() {
        let request: DeviceRequest =
            serde_json::from_str(r#"{"id": "gpu:0"}"#).unwrap();
        assert_eq!(request.mode, DeviceAccessMode::Compute);
    }

    #[test]
    fn parses_modes() {
        assert_eq!("full".parse::<DeviceAccessMode>().unwrap(), DeviceAccessMode::Full);
        assert!("wireless".parse::<DeviceAccessMode>().is_err());
    }
}
