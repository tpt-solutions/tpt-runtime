//! # tpt-runtime-core
//!
//! Platform-independent primitives shared by every tpt-runtime crate:
//! identifiers ([`id`]), the workload lifecycle state machine ([`state`]),
//! errors ([`error`]), events ([`event`]) and timestamps.
//!
//! This crate must stay free of platform dependencies (SPEC §8): no Windows,
//! WASM or OCI specific concepts belong here.

pub mod error;
pub mod event;
pub mod id;
pub mod state;
pub mod timestamp;
pub mod usage;

pub use error::{RuntimeError, Result};
pub use event::{EventKind, RuntimeEvent};
pub use id::{CapabilityId, DeviceId, NetworkId, ResourceId, ServiceId, VolumeId, WorkloadId};
pub use state::{WorkloadState, LIFECYCLE_TRANSITIONS};
pub use timestamp::Timestamp;
pub use usage::ResourceUsage;
