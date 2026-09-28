//! # tpt-runtime-linux
//!
//! Linux execution environments (SPEC §11) — the layered strategy:
//!
//! - **Phase 1 (this crate today)**: run Linux workloads inside *existing*
//!   Windows virtualization, i.e. WSL (`wsl.exe`), exposing them through the
//!   common workload model. The runtime distinguishes Linux *compatibility*
//!   from Linux *implementation* (SPEC §11): a workload only needs a
//!   Linux-compatible execution environment.
//! - **Phase 2**: tighter TPT resource management integration.
//! - **Phase 3**: a TPT-managed Linux execution environment.
//!
//! The backend shells out to `wsl.exe` with distro names from the manifest
//! (`[execution] backend = "linux"`, `distro = "ubuntu"`). Isolation
//! boundaries inside the distro are out of scope here; capability grants
//! are recorded and audited by the runtime.

pub mod backend;

pub use backend::LinuxBackend;
