//! Logical volume mounts (SPEC §15).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Access mode of a volume mount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAccessMode {
    /// Read-only view of the volume.
    ReadOnly,
    /// Read-write view of the volume.
    #[default]
    ReadWrite,
}

impl std::str::FromStr for VolumeAccessMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "read-only" | "readonly" | "ro" => Ok(VolumeAccessMode::ReadOnly),
            "read-write" | "readwrite" | "rw" => Ok(VolumeAccessMode::ReadWrite),
            other => Err(format!("unknown volume mode '{other}' (expected read-only or read-write)")),
        }
    }
}

impl fmt::Display for VolumeAccessMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VolumeAccessMode::ReadOnly => f.write_str("read-only"),
            VolumeAccessMode::ReadWrite => f.write_str("read-write"),
        }
    }
}

/// One volume mount: binds a named logical volume into the workload at a
/// mount point (SPEC §15, §32 `[[volumes]]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeMount {
    /// Logical volume name, resolvable through the storage subsystem.
    pub name: String,
    /// Path inside the workload where the volume appears (e.g. `/workspace`).
    pub mount: String,
    /// Access mode; defaults to read-write.
    #[serde(default)]
    pub mode: VolumeAccessMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_access_modes() {
        assert_eq!("read-only".parse::<VolumeAccessMode>().unwrap(), VolumeAccessMode::ReadOnly);
        assert_eq!("rw".parse::<VolumeAccessMode>().unwrap(), VolumeAccessMode::ReadWrite);
        assert!("append".parse::<VolumeAccessMode>().is_err());
    }

    #[test]
    fn mounts_round_trip_through_toml_style_json() {
        let mount = VolumeMount {
            name: "source".to_owned(),
            mount: "/workspace".to_owned(),
            mode: VolumeAccessMode::ReadOnly,
        };
        let value: serde_json::Value = serde_json::to_value(&mount).unwrap();
        assert_eq!(value["name"], "source");
        assert_eq!(value["mode"], "read-only");
    }
}
