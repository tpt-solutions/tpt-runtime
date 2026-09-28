//! Per-backend execution payloads (SPEC §10, §12–§14).
//!
//! The workload model stays independent of the backend: exactly one
//! [`ExecutionSpec`] variant is populated for a workload, selected by the
//! manifest's `backend` key. The variants only carry *requirements*; how a
//! backend realizes them (job objects, WASI preopens, OCI bundles, ...) is
//! that backend's business.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// Backend-specific execution definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSpec {
    /// Native Windows process (SPEC §12).
    WindowsProcess(WindowsProcessSpec),
    /// Linux execution environment (SPEC §11).
    LinuxProcess(LinuxProcessSpec),
    /// OCI image (SPEC §13).
    OciImage(OciImageSpec),
    /// WASM module (SPEC §14).
    WasmModule(WasmModuleSpec),
}

impl ExecutionSpec {
    /// The backend that executes this workload.
    pub fn backend(&self) -> crate::backend::BackendKind {
        match self {
            ExecutionSpec::WindowsProcess(_) => crate::backend::BackendKind::Windows,
            ExecutionSpec::LinuxProcess(_) => crate::backend::BackendKind::Linux,
            ExecutionSpec::OciImage(_) => crate::backend::BackendKind::Oci,
            ExecutionSpec::WasmModule(_) => crate::backend::BackendKind::Wasm,
        }
    }

    /// Validates backend-independent invariants shared by all variants.
    pub fn validate(&self) -> Result<()> {
        match self {
            ExecutionSpec::WindowsProcess(spec) => spec.validate(),
            ExecutionSpec::LinuxProcess(spec) => spec.validate(),
            ExecutionSpec::OciImage(spec) => spec.validate(),
            ExecutionSpec::WasmModule(spec) => spec.validate(),
        }
    }
}

/// A native Windows process: an executable plus arguments and environment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowsProcessSpec {
    /// Path or name of the executable to launch.
    #[serde(default)]
    pub program: String,
    /// Arguments passed to the executable.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory; defaults to the first read-write volume mount or
    /// the daemon's runtime directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    /// Environment variables set on top of a sanitized default environment.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl WindowsProcessSpec {
    fn validate(&self) -> Result<()> {
        if self.program.trim().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "windows backend requires a program to execute",
            ));
        }
        Ok(())
    }
}

/// A Linux execution environment. Resolution is delegated to the Linux
/// backend (WSL integration first; see SPEC §11).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinuxProcessSpec {
    /// Distro/environment name, e.g. `ubuntu`.
    #[serde(default)]
    pub distro: String,
    /// Command to execute inside the Linux environment.
    #[serde(default)]
    pub command: Vec<String>,
    /// Environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl LinuxProcessSpec {
    fn validate(&self) -> Result<()> {
        if self.distro.trim().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "linux backend requires a distro name",
            ));
        }
        Ok(())
    }
}

/// An OCI image reference to resolve and run (SPEC §13).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OciImageSpec {
    /// Image reference, e.g. `postgres:16` or `ghcr.io/org/app@sha256:...`.
    #[serde(default)]
    pub image: String,
    /// Entry point override; uses the image default when absent.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl OciImageSpec {
    fn validate(&self) -> Result<()> {
        if self.image.trim().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "oci backend requires an image reference",
            ));
        }
        oci_ref::ImageReference::parse(&self.image)
            .map(|_| ())
            .map_err(|err| {
                RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("invalid oci image reference: {err}"),
                )
            })
    }
}

/// A WASM module and its WASI configuration (SPEC §14).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WasmModuleSpec {
    /// Path to the `.wasm` module (WAT text is accepted by the backend).
    #[serde(default)]
    pub module: PathBuf,
    /// Function class of the workload (SPEC §14 runtime classes).
    #[serde(default)]
    pub class: WasmClass,
    /// Environment variables visible through WASI.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Arguments passed to the module (`args[0]` is the module name).
    #[serde(default)]
    pub args: Vec<String>,
}

/// Runtime class of a WASM workload (SPEC §14).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WasmClass {
    /// One-shot command (`wasm-command`).
    #[default]
    Command,
    /// Long-running service (`wasm-service`).
    Service,
    /// Pure function invocation (`wasm-function`).
    Function,
}

impl WasmModuleSpec {
    fn validate(&self) -> Result<()> {
        if self.module.as_os_str().is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "wasm backend requires a module path",
            ));
        }
        Ok(())
    }
}

/// OCI image reference parsing (`name[:tag][@digest]`), shared by the OCI
/// backend and manifest validation. Kept here (not in `tpt-runtime-oci`) so
/// the manifest can validate references without pulling in image handling.
pub mod oci_ref {
    /// A parsed OCI image reference.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ImageReference {
        /// Repository path, e.g. `library/postgres`.
        pub repository: String,
        /// Tag, defaults to `latest` when absent.
        pub tag: Option<String>,
        /// Content digest (`sha256:...`) when pinned.
        pub digest: Option<String>,
    }

    impl ImageReference {
        /// Parses `name[:tag][@digest]` references, optionally including a
        /// registry host (`ghcr.io/org/app:1.4.0`, `localhost:5000/app`).
        pub fn parse(reference: &str) -> Result<ImageReference, String> {
            let reference = reference.trim();
            if reference.is_empty() {
                return Err("empty reference".to_owned());
            }

            let (rest, digest) = match reference.split_once('@') {
                Some((r, d)) => (r, Some(d.to_owned())),
                None => (reference, None),
            };
            if let Some(digest) = &digest {
                if !digest.starts_with("sha256:") || digest.len() != "sha256:".len() + 64 {
                    return Err(format!("unsupported digest '{digest}'"));
                }
            }

            // A trailing `:tag` only exists when the text after the last
            // colon contains no slash (ports like `localhost:5000/app`
            // therefore stay inside the repository).
            let (repository, tag) = match rest.rsplit_once(':') {
                Some((repo, tag)) if !tag.contains('/') => {
                    if tag.is_empty() {
                        return Err("empty tag".to_owned());
                    }
                    (repo, Some(tag.to_owned()))
                }
                _ => (rest, None),
            };

            if repository.is_empty() {
                return Err("empty repository path".to_owned());
            }

            Ok(ImageReference {
                repository: repository.to_owned(),
                tag,
                digest,
            })
        }
    }

    impl std::fmt::Display for ImageReference {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.repository)?;
            if let Some(tag) = &self.tag {
                write!(f, ":{tag}")?;
            }
            if let Some(digest) = &self.digest {
                write!(f, "@{digest}")?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_name_tag() {
            let r = ImageReference::parse("postgres:16").unwrap();
            assert_eq!(r.repository, "postgres");
            assert_eq!(r.tag.as_deref(), Some("16"));
            assert_eq!(r.digest, None);
            assert_eq!(r.to_string(), "postgres:16");
        }

        #[test]
        fn defaults_no_tag() {
            let r = ImageReference::parse("postgres").unwrap();
            assert_eq!(r.repository, "postgres");
            assert_eq!(r.tag, None);
        }

        #[test]
        fn parses_registry_host_and_digest() {
            let r = ImageReference::parse("ghcr.io/org/app:1.4.0@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
            assert_eq!(r.repository, "ghcr.io/org/app");
            assert_eq!(r.tag.as_deref(), Some("1.4.0"));
            assert!(r.digest.as_deref().unwrap().starts_with("sha256:"));
        }

        #[test]
        fn rejects_bad_digest_and_empty() {
            assert!(ImageReference::parse("app@md5:deadbeef").is_err());
            assert!(ImageReference::parse("").is_err());
            assert!(ImageReference::parse("app:").is_err());
        }
    }
}
