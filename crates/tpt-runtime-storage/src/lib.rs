//! # tpt-runtime-storage
//!
//! Logical volumes (SPEC §15): named, inspectable storage units that
//! workloads mount as views. The MVP implementation is directory-backed;
//! Archon-backed volumes (SPEC §16) would implement the same [`Volume`]
//! model.
//!
//! ```text
//! tpt volume create project
//! tpt volume mount project /workspace
//! ```

#[cfg(feature = "archon")]
pub mod archon;
pub mod volume;

pub use volume::{StorageManager, Volume, VolumeInfo};
