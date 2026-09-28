//! # tpt-runtime-device
//!
//! The device model (SPEC §19): devices are capabilities. A workload asks
//! for `gpu:0` or `serial:com3` by logical id, and the runtime decides how
//! the access is realized on this host. The registry tracks which logical
//! devices exist and which are claimed.

pub mod registry;

pub use registry::{DeviceClass, DeviceInfo, DeviceRegistry};
