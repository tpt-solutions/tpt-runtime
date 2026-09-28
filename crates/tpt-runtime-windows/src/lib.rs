//! # tpt-runtime-windows
//!
//! The native Windows process backend (SPEC §12): first-class Windows
//! workloads with process-group semantics via Job Objects, environment
//! management, stdout/stderr capture and resource accounting.
//!
//! Isolation notes (SPEC §23):
//!
//! - Every workload process runs inside a dedicated Job Object; termination
//!   is job-wide, so child processes cannot outlive the workload.
//! - `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ties workload lifetime to the
//!   daemon's lifetime: a crashed daemon cannot leak workload processes.
//! - Memory limits are enforced by the job (`ProcessMemoryLimit`) when the
//!   manifest requests one.
//! - The backend intentionally offers no ambient host access: env is a
//!   sanitized allowlist plus manifest values.
//!
//! On non-Windows hosts the crate compiles but reports
//! `backend_unavailable`.

pub mod backend;

pub use backend::WindowsProcessBackend;
