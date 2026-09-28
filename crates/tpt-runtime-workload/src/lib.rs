//! # tpt-runtime-workload
//!
//! The workload manager (SPEC §9, §26): one coherent lifecycle over every
//! execution backend. It enforces the state machine from
//! `tpt-runtime-core`, resolves resources (volumes, networks, devices)
//! before start, records capability grants, and normalizes events and
//! metrics through `tpt-runtime-observe`.
//!
//! The manager is the only component that knows all backends; everything
//! above it (API, daemon, CLI) speaks in workload terms only, keeping the
//! workload model backend-independent (SPEC §5.3).

pub mod info;
pub mod manager;

pub use info::WorkloadInfo;
pub use manager::{LogsQuery, WorkloadManager};
