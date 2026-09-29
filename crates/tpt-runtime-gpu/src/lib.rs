//! # tpt-runtime-gpu
//!
//! GPU support (SPEC §20): discovery and telemetry for the "Windows host +
//! NVIDIA GPU" MVP target. GPUs are registered into the device registry as
//! `gpu:<index>` devices; granting one to a workload is a capability
//! decision, never ambient.
//!
//! NVIDIA integration reads `nvidia-smi`; a host without it simply has no
//! GPUs (that is discovery, not failure — SPEC §48 distinguishes the two).

pub mod discover;
pub mod telemetry;

pub use discover::{discover_gpus, GpuInfo};
pub use telemetry::{sample_gpus, GpuSample, GpuTelemetry};
