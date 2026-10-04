//! # tpt-runtime-policy
//!
//! Declarative resource policies (SPEC §25) and admission decisions: which
//! resource requests are granted, clamped or denied.
//!
//! The engine is intentionally pure: it takes host capacity plus a request
//! and returns decisions; applying decisions is the backends' job.

pub mod host;
pub mod policy;

pub use policy::{HostCapacity, PolicyDecision, PolicyEngine, ResourceClass, ResourcePolicy};
