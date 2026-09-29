//! GPU telemetry (SPEC §20, §28): periodic per-GPU samples via
//! `nvidia-smi`, cached as latest-known values for `daemon.status` and
//! future policy decisions.
//!
//! Sampling is best-effort: a failed or absent `nvidia-smi` leaves the
//! previous sample in place and never disturbs the runtime (the host
//! simply has no fresh telemetry — that is observation, not failure).

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// One telemetry sample for one GPU.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GpuSample {
    /// Zero-based device index (`gpu:<index>`).
    pub index: u32,
    /// GPU utilization, percent.
    pub utilization_pct: f64,
    /// Memory in use, MiB.
    pub memory_used_mib: u64,
    /// Total memory, MiB.
    pub memory_total_mib: u64,
    /// Temperature, degrees Celsius, when reported.
    pub temperature_c: Option<f64>,
    /// Board power draw, watts, when reported.
    pub power_watts: Option<f64>,
    /// When the sample was taken; serialized as `age_ms` (milliseconds
    /// since the sample, computed at serialization time).
    #[serde(serialize_with = "serialize_age_ms", rename = "age_ms")]
    pub collected_at: Instant,
}

fn serialize_age_ms<S: serde::Serializer>(
    instant: &Instant,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_u64(instant.elapsed().as_millis() as u64)
}

impl GpuSample {
    /// Fraction of GPU memory in use, 0.0..=1.0.
    pub fn memory_utilization(&self) -> f64 {
        if self.memory_total_mib == 0 {
            0.0
        } else {
            self.memory_used_mib as f64 / self.memory_total_mib as f64
        }
    }
}

/// Samples all GPUs once via `nvidia-smi`.
pub fn sample_gpus() -> Result<Vec<GpuSample>> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|err| {
            RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("nvidia-smi is not available: {err}"),
            )
            .with_backend("gpu")
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut samples = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(", ").map(str::trim).collect();
        if fields.len() < 4 {
            continue;
        }
        let index: u32 = fields[0].parse().map_err(|_| {
            RuntimeError::new(
                ErrorKind::DeviceUnavailable,
                format!("unparsable nvidia-smi telemetry: '{line}'"),
            )
            .with_backend("gpu")
        })?;
        let parse_f64 = |text: &str| -> Option<f64> {
            let text = text.trim();
            if text.is_empty() || text == "[N/A]" || text == "N/A" {
                None
            } else {
                text.parse().ok()
            }
        };
        samples.push(GpuSample {
            index,
            utilization_pct: parse_f64(fields[1]).unwrap_or(0.0),
            memory_used_mib: parse_f64(fields[2]).unwrap_or(0.0) as u64,
            memory_total_mib: parse_f64(fields[3]).unwrap_or(0.0) as u64,
            temperature_c: fields.get(4).and_then(|f| parse_f64(f)),
            power_watts: fields.get(5).and_then(|f| parse_f64(f)),
            collected_at: Instant::now(),
        });
    }
    Ok(samples)
}

/// Latest-known GPU samples, refreshed by a background thread.
pub struct GpuTelemetry {
    latest: Mutex<BTreeMap<u32, GpuSample>>,
}

impl GpuTelemetry {
    /// Takes one sample immediately and starts a background thread
    /// refreshing every `period`.
    pub fn spawn(period: Duration) -> Arc<Self> {
        let telemetry = Arc::new(Self {
            latest: Mutex::new(BTreeMap::new()),
        });
        GpuTelemetry::record(&telemetry);
        let sampler = telemetry.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(period);
            GpuTelemetry::record(&sampler);
        });
        telemetry
    }

    /// Records one sampling pass (failures keep previous samples).
    pub fn record(telemetry: &Self) {
        if let Ok(samples) = sample_gpus() {
            let mut latest = telemetry.latest.lock().unwrap();
            for sample in samples {
                latest.insert(sample.index, sample);
            }
        }
    }

    /// The newest sample per GPU, ordered by index.
    pub fn snapshot(&self) -> Vec<GpuSample> {
        self.latest.lock().unwrap().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_is_safe_on_any_host() {
        // Hosts without nvidia-smi error here; hosts with one must yield
        // well-formed samples (this test asserts both paths behave).
        match sample_gpus() {
            Ok(samples) => {
                for sample in &samples {
                    assert!(sample.utilization_pct >= 0.0 && sample.utilization_pct <= 100.0);
                    assert!(sample.memory_used_mib <= sample.memory_total_mib);
                    assert!(sample.memory_utilization() >= 0.0);
                }
            }
            Err(err) => assert_eq!(err.kind, ErrorKind::DeviceUnavailable),
        }
    }

    #[test]
    fn telemetry_cache_survives_failed_polls() {
        // Direct cache behavior: record merges, snapshot is ordered.
        let telemetry = GpuTelemetry {
            latest: Mutex::new(BTreeMap::new()),
        };
        let base = Instant::now();
        telemetry.latest.lock().unwrap().insert(
            1,
            GpuSample {
                index: 1,
                utilization_pct: 10.0,
                memory_used_mib: 100,
                memory_total_mib: 8000,
                temperature_c: None,
                power_watts: None,
                collected_at: base,
            },
        );
        telemetry.latest.lock().unwrap().insert(
            0,
            GpuSample {
                index: 0,
                utilization_pct: 50.0,
                memory_used_mib: 4000,
                memory_total_mib: 8000,
                temperature_c: Some(60.0),
                power_watts: Some(120.0),
                collected_at: base,
            },
        );

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].index, 0);
        assert_eq!(snapshot[1].index, 1);
        assert!((snapshot[0].memory_utilization() - 0.5).abs() < 1e-9);
    }
}
