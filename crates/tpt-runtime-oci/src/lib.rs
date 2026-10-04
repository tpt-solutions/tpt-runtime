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
//!   hands to an isolation provider (runtime-spec `config.json` included).
//! - [`store::ImageStore`] — content-addressed image store: raw blobs
//!   (manifests, configs, layers) plus unpacked bundles, pinned by refs.
//! - [`registry::RegistryClient`] — Docker Registry HTTP API v2 client
//!   (anonymous Bearer token flow, manifest/index resolution, streaming
//!   blob downloads with sha256 verification and size caps).
//! - [`unpack::unpack_layer`] — tar/tar+gzip layer extraction with
//!   overlay whiteouts and hostile-entry rejection (traversal, symlink
//!   escapes, per-file caps).
//! - [`image::pull`] — the pipeline: manifest → config → layers → rootfs
//!   → bundle (cached; layers already in the store are not re-downloaded).
//! - [`OciBackend`] — the [`ExecutionBackend`] implementation with a
//!   configurable pull policy; `start` reports `not_implemented` until an
//!   isolation provider exists (see `tpt-boxcar` / Origin, SPEC §17).

pub mod backend;
pub mod bundle;
pub mod image;
pub mod registry;
pub mod store;
pub mod unpack;

pub use backend::{OciBackend, PullPolicy};
pub use bundle::Bundle;
pub use registry::{
    digest_of, verify_bytes, ImageConfig, ImageIndex, ImageManifest, RegistryClient,
};
pub use store::{ImageDefaults, ImageStore};
pub use unpack::unpack_layer;
