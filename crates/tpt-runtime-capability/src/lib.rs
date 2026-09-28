//! # tpt-runtime-capability
//!
//! The capability model (SPEC §5.2, §23): workloads receive only the access
//! that is explicitly granted; ambient host authority does not exist.
//!
//! Capabilities are declared by dotted names in manifests
//! (`network.outbound`, `filesystem.write`, `device:gpu:0`, `secret:token`,
//! `ipc:service:database`) and parsed into typed [`Capability`] values that
//! backends can enforce. Every check outcome is attributable
//! (`workload → identity → capability → resource → operation`, SPEC §23).
//!
//! Revocation: a [`CapabilitySet`] can drop grants at any time; backends are
//! expected to consult the set before performing privileged work.

pub mod capability;

pub use capability::{Capability, CapabilityCheck, CapabilitySet};
