//! # tpt-runtime-process
//!
//! The execution backend interface (SPEC §10) plus the runtime pieces every
//! backend needs: the prepared/start contexts, the [`WorkloadInstance`]
//! handle a backend returns, exit statuses and log capture buffers.
//!
//! Concrete backends live in their own crates (`tpt-runtime-windows`,
//! `tpt-runtime-wasm`, `tpt-runtime-oci`, `tpt-runtime-linux`) and implement
//! [`ExecutionBackend`]. The workload manager ([`tpt-runtime-workload`])
//! consumes the trait, never the concrete types, keeping the workload model
//! backend-independent (SPEC §5.3).
//!
//! ```text
//! trait ExecutionBackend {
//!     fn prepare(&self, spec, ctx) -> Result<()>;
//!     fn start(&self, id, spec, ctx) -> Result<Box<dyn WorkloadInstance>>;
//! }
//! ```

pub mod backend;
pub mod logs;

pub use backend::{
    ExitStatus, ExecutionBackend, ResolvedMount, StartContext, StopMode, WorkloadInstance,
};
pub use logs::LogCapture;
