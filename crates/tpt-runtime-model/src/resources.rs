//! Resource requests and limits (SPEC §25).

use serde::{Deserialize, Serialize};
use std::fmt;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// A byte amount that also parses human sizes (`512MiB`, `4GiB`, `1KiB`,
/// plain integers as bytes). Serialization uses plain bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Memory(pub u64);

impl serde::Serialize for Memory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Memory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Memory;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a byte count (integer) or a size string such as \"4GiB\"")
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Memory, E> {
                Ok(Memory(value))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Memory, E> {
                if value < 0 {
                    return Err(E::invalid_value(
                        serde::de::Unexpected::Signed(value),
                        &"a non-negative byte count",
                    ));
                }
                Ok(Memory(value as u64))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Memory, E> {
                value
                    .parse::<Memory>()
                    .map_err(|_| E::invalid_value(serde::de::Unexpected::Str(value), &self))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

impl Memory {
    /// 1 KiB = 1024 bytes.
    pub const KIB: u64 = 1024;
    /// 1 MiB = 1024 KiB.
    pub const MIB: u64 = Self::KIB * 1024;
    /// 1 GiB = 1024 MiB.
    pub const GIB: u64 = Self::MIB * 1024;

    /// Convenience constructor for whole GiB.
    pub fn gib(value: u64) -> Self {
        Self(value * Self::GIB)
    }

    /// Convenience constructor for whole MiB.
    pub fn mib(value: u64) -> Self {
        Self(value * Self::MIB)
    }
}

impl std::str::FromStr for Memory {
    type Err = RuntimeError;

    fn from_str(text: &str) -> Result<Self> {
        let text = text.trim();
        let digits = text.trim_end_matches(|c: char| !c.is_ascii_digit());
        let (number, unit) = text.split_at(digits.len());
        let number: u64 = number
            .parse()
            .map_err(|_| RuntimeError::new(ErrorKind::InvalidConfiguration, format!("invalid memory size '{text}'")))?;
        let multiplier = match unit.trim().to_ascii_uppercase().as_str() {
            "" | "B" | "BYTES" => 1,
            "K" | "KB" | "KIB" => Memory::KIB,
            "M" | "MB" | "MIB" => Memory::MIB,
            "G" | "GB" | "GIB" => Memory::GIB,
            other => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("unknown memory unit '{other}' in '{text}'"),
                ))
            }
        };
        number
            .checked_mul(multiplier)
            .map(Memory)
            .ok_or_else(|| RuntimeError::new(ErrorKind::InvalidConfiguration, format!("memory size '{text}' overflows")))
    }
}

impl fmt::Display for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 % Memory::GIB == 0 && self.0 >= Memory::GIB {
            write!(f, "{}GiB", self.0 / Memory::GIB)
        } else if self.0 % Memory::MIB == 0 && self.0 >= Memory::MIB {
            write!(f, "{}MiB", self.0 / Memory::MIB)
        } else {
            write!(f, "{}B", self.0)
        }
    }
}

/// Resource requests for a workload (SPEC §25).
///
/// Every field is optional: absent means "backend default, no explicit
/// limit". Requests and actual usage are tracked separately by the runtime
/// (see `tpt-runtime-observe`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceSpec {
    /// CPU cores made available to the workload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f64>,
    /// Memory limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
    /// Wall-clock limit in seconds; the workload is stopped when exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// WASM fuel limit: an upper bound on executed instructions for
    /// deterministic CPU bounding of WASM workloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel: Option<u64>,
}

impl ResourceSpec {
    /// Validates the request (positive values, sane sizes).
    pub fn validate(&self) -> Result<()> {
        if let Some(cpu) = self.cpu {
            if !(cpu > 0.0) || cpu > 1024.0 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("cpu request {cpu} out of range (0, 1024]"),
                ));
            }
        }
        if let Some(memory) = self.memory {
            if memory.0 == 0 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    "memory request must be greater than zero",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_sizes() {
        assert_eq!("4GiB".parse::<Memory>().unwrap(), Memory::gib(4));
        assert_eq!("512MiB".parse::<Memory>().unwrap(), Memory::mib(512));
        assert_eq!("64KiB".parse::<Memory>().unwrap(), Memory(64 * Memory::KIB));
        assert_eq!("100".parse::<Memory>().unwrap(), Memory(100));
        assert_eq!("2 GB".parse::<Memory>().unwrap(), Memory(2 * Memory::GIB));
    }

    #[test]
    fn rejects_bad_sizes() {
        assert!("4TiB".parse::<Memory>().is_err());
        assert!("lots".parse::<Memory>().is_err());
        assert!("".parse::<Memory>().is_err());
    }

    #[test]
    fn displays_compactly() {
        assert_eq!(Memory::gib(4).to_string(), "4GiB");
        assert_eq!(Memory::mib(512).to_string(), "512MiB");
        assert_eq!(Memory(42).to_string(), "42B");
    }

    #[test]
    fn serializes_as_plain_bytes() {
        assert_eq!(serde_json::to_string(&Memory::gib(1)).unwrap(), "1073741824");
    }

    #[test]
    fn validates_resource_requests() {
        let mut spec = ResourceSpec {
            cpu: Some(0.0),
            ..Default::default()
        };
        assert!(spec.validate().is_err());
        spec.cpu = Some(4.0);
        spec.memory = Some(Memory::gib(8));
        assert!(spec.validate().is_ok());
    }
}
