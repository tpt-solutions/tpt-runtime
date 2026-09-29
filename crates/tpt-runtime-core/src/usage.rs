//! Resource usage accounting (SPEC §25, §27).

use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Observed resource consumption of a workload.
///
/// Backends fill this from their platform mechanisms (job objects on
/// Windows, engine counters for WASM); the runtime exposes requested values
/// alongside actual usage (SPEC §25).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceUsage {
    /// User-mode CPU time consumed.
    #[serde(
        default,
        with = "duration_nanos",
        skip_serializing_if = "Duration::is_zero"
    )]
    pub user_cpu: Duration,
    /// Kernel-mode CPU time consumed.
    #[serde(
        default,
        with = "duration_nanos",
        skip_serializing_if = "Duration::is_zero"
    )]
    pub kernel_cpu: Duration,
    /// Peak memory observed, bytes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub memory_peak_bytes: u64,
    /// Bytes read from storage.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub read_bytes: u64,
    /// Bytes written to storage or output streams.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub write_bytes: u64,
    /// Live processes in the workload (native backends).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub process_count: u32,
    /// When this snapshot was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collected_at: Option<Timestamp>,
}

fn is_zero<T: PartialEq + Default>(value: &T) -> bool {
    *value == T::default()
}

mod duration_nanos {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        value.as_nanos().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let nanos = u128::deserialize(deserializer)?;
        Ok(Duration::from_nanos(u64::try_from(nanos).map_err(
            |_| serde::de::Error::custom("duration nanos overflow u64"),
        )?))
    }
}

impl ResourceUsage {
    /// Total CPU time (user + kernel).
    pub fn total_cpu(&self) -> Duration {
        self.user_cpu + self.kernel_cpu
    }

    /// Returns the larger of the two snapshots field by field; used for
    /// peak-style counters.
    pub fn max_with(&self, other: &ResourceUsage) -> ResourceUsage {
        ResourceUsage {
            user_cpu: self.user_cpu.max(other.user_cpu),
            kernel_cpu: self.kernel_cpu.max(other.kernel_cpu),
            memory_peak_bytes: self.memory_peak_bytes.max(other.memory_peak_bytes),
            read_bytes: self.read_bytes.max(other.read_bytes),
            write_bytes: self.write_bytes.max(other.write_bytes),
            process_count: self.process_count.max(other.process_count),
            collected_at: self.collected_at.or(other.collected_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_compactly_when_empty() {
        let value: serde_json::Value = serde_json::to_value(ResourceUsage::default()).unwrap();
        assert!(value.as_object().unwrap().is_empty());
    }

    #[test]
    fn durations_round_trip_as_nanos() {
        let usage = ResourceUsage {
            user_cpu: Duration::from_millis(1500),
            memory_peak_bytes: 4096,
            ..Default::default()
        };
        let value: serde_json::Value = serde_json::to_value(&usage).unwrap();
        assert_eq!(value["user_cpu"], 1_500_000_000u64);
        let back: ResourceUsage = serde_json::from_value(value).unwrap();
        assert_eq!(back, usage);
    }

    #[test]
    fn max_with_takes_peaks() {
        let a = ResourceUsage {
            user_cpu: Duration::from_secs(1),
            memory_peak_bytes: 100,
            ..Default::default()
        };
        let b = ResourceUsage {
            user_cpu: Duration::from_secs(2),
            memory_peak_bytes: 50,
            write_bytes: 10,
            ..Default::default()
        };
        let peak = a.max_with(&b);
        assert_eq!(peak.user_cpu, Duration::from_secs(2));
        assert_eq!(peak.memory_peak_bytes, 100);
        assert_eq!(peak.write_bytes, 10);
    }
}
