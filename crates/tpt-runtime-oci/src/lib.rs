//! # tpt-runtime-oci
//!
//! OCI support (SPEC §13): images are a *compatibility format*, not the
//! fundamental architecture. The backend is built on Boxcar-compatible
//! primitives — image resolution, layer storage, bundle preparation — which
//! `tpt-boxcar` is slated to provide (SPEC §17).
//!
//! What exists today:
//!
//! - [`bundle::Bundle`] — the OCI bundle model the runtime prepares and
//!   hands to an isolation provider.
//! - [`store::ImageStore`] — content-addressed image directory layout
//!   (digest → unpacked bundle) with resolution policies.
//! - [`OciBackend`] — the [`ExecutionBackend`] implementation; it prepares
//!   bundles from the image store and reports `not_implemented` when an
//!   operation needs Boxcar primitives that do not exist yet.
//!
//! What is intentionally *not* here yet: registry protocol client, layer
//! unpacking (tar/whiteouts), namespace/network plumbing. Those arrive with
//! the Boxcar integration (Phase 4 of the roadmap).

pub mod backend;
pub mod bundle;
pub mod store;

pub use backend::OciBackend;
pub use bundle::Bundle;
pub use store::ImageStore;
