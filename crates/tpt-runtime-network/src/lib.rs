//! # tpt-runtime-network
//!
//! Logical networks (SPEC §18): workloads request an *intent* — `none`,
//! `host`, `private`, `service`, `isolated`, `outbound` — and the runtime
//! resolves the mechanism. Users never configure adapters, NAT or bridges
//! for ordinary workloads.
//!
//! MVP behavior on Windows:
//!
//! - `none`/`isolated`/`outbound`: realized for WASM workloads by WASI
//!   socket grants (deny by default); native Windows processes run with the
//!   host stack and rely on policy + auditing (documented limitation).
//! - `service`/`host`: inbound exposure is realized by allocating host
//!   ports ([`NetworkManager::assign`]).
//!
//! Future Boxcar service-mesh integration plugs in behind the same
//! [`NetworkAssignment`] model.

pub mod manager;

pub use manager::{NetworkAssignment, NetworkManager, PortAllocation};
