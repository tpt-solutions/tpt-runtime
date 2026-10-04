//! # tpt-runtime-sandbox
//!
//! The runtime's own OCI isolation provider (SPEC §13, §17): the piece that
//! turns a prepared [`Bundle`] into a running, isolated Windows process.
//!
//! `tpt-boxcar` was the intended provider, but its Origin sandbox does not
//! spawn containers (`type: oci` is documented as bookkeeping-only) and has
//! no Windows isolation, so this crate provides that boundary natively. It
//! sits *behind* [`SandboxProvider`], so Boxcar — or any other provider —
//! can still be dropped in later without touching `tpt-runtime-oci`.
//!
//! What the Windows provider actually enforces:
//!
//! - **Job Objects** — kill-on-close ties workload lifetime to the daemon's,
//!   memory limits are enforced, and accounting is read back from the job.
//! - **Restricted tokens** — every workload runs from a token created with
//!   `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)`, so no privilege the
//!   daemon holds is inheritable by the image.
//! - **Handle inheritance lists** — only the two log pipes are inheritable,
//!   so an image cannot reach the daemon's open handles.
//! - **A sanitized environment** — the daemon's environment never leaks in;
//!   the image gets a minimal Windows environment plus its own `config.json`
//!   env and the manifest's env.
//! - **Rootfs-scoped entry points** — the OCI argv's `argv[0]` is resolved
//!   inside the unpacked rootfs and refused if it escapes.
//!
//! What it deliberately does **not** claim: this is process-level isolation,
//! not a container filesystem. Windows has no `chroot`, so the rootfs is a
//! *source tree* for the entry point, not a jail — an image can still read
//! the host filesystem. Namespace-level confinement needs Hyper-V/WSL and is
//! out of scope. See the crate README.
//!
//! ```text
//! OciBackend::prepare  ->  Bundle { path, args, env }
//!                          |
//!                          v
//!              SandboxProvider::start
//!                          |
//!         restricted token + job object + log pipes
//!                          |
//!                          v
//!                 WorkloadInstance
//! ```

pub mod bundle_env;
pub mod mounts;
pub mod network;
pub mod provider;
pub mod token;

#[cfg(windows)]
mod job;
#[cfg(windows)]
mod win32;

pub use provider::{SandboxLimits, SandboxProvider, WindowsSandbox};

#[cfg(windows)]
pub use job::{JobObject, JobUsage};
#[cfg(windows)]
pub use token::RestrictedToken;
