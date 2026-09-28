//! # tpt-runtime-config
//!
//! Declarative configuration (SPEC §31, §32): the `tpt.runtime/v1` workload
//! manifest parsed from TOML, plus daemon settings resolved from the
//! environment.
//!
//! ```toml
//! api = "tpt.runtime/v1"
//!
//! [workload]
//! name = "example"
//!
//! [execution]
//! backend = "windows"
//! program = "myapp.exe"
//!
//! [resources]
//! cpu = 4
//! memory = "4GiB"
//!
//! [network]
//! mode = "service"
//!
//! [network.expose]
//! http = 8080
//!
//! [[volumes]]
//! name = "source"
//! mount = "/workspace"
//! mode = "read-write"
//!
//! [[capabilities]]
//! name = "network.outbound"
//! ```

pub mod daemon;
pub mod manifest;

pub use daemon::{DaemonConfig, DEFAULT_PIPE_NAME};
pub use manifest::{Manifest, MANIFEST_API_VERSION};
