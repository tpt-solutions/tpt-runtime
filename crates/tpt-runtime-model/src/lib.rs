//! # tpt-runtime-model
//!
//! The backend-independent workload model (SPEC §9): what a workload *is* —
//! its execution requirements, resources, volumes, networks, devices,
//! environment and capabilities — without saying anything about *how* a
//! particular backend realizes them (SPEC §5.3, §10).
//!
//! Backends: [`BackendKind`]. Execution payloads: [`ExecutionSpec`].
//! Resources: [`ResourceSpec`]. Auxiliary models: [`VolumeMount`],
//! [`NetworkSpec`], [`DeviceRequest`].

pub mod backend;
pub mod device;
pub mod execution;
pub mod network;
pub mod resources;
pub mod volume;
pub mod workload;

pub use backend::BackendKind;
pub use device::{DeviceAccessMode, DeviceRequest};
pub use execution::ExecutionSpec;
pub use network::{NetworkMode, NetworkSpec};
pub use resources::{Memory, ResourceSpec};
pub use volume::{VolumeAccessMode, VolumeMount};
pub use workload::WorkloadSpec;
