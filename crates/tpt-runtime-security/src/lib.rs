//! # tpt-runtime-security
//!
//! Secrets (SPEC §24) and audit attribution (SPEC §23).
//!
//! Secrets are never ordinary environment variables by default: a workload
//! receives an ephemeral capability ([`Capability::Secret`]), and values are
//! only released through [`SecretStore::resolve`] — which *requires* the
//! matching capability as authorization. Values never appear in API
//! responses ([`SecretSummary`] redacts them) and the store keeps values
//! out of logs.

pub mod secrets;

pub use secrets::{SecretStore, SecretSummary};
