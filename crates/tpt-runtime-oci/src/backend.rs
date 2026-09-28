//! OCI execution backend on Boxcar-compatible primitives (SPEC §13, §17).

use std::path::PathBuf;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::execution::oci_ref::ImageReference;
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, StartContext, WorkloadInstance};

use crate::bundle::Bundle;
use crate::store::ImageStore;

/// OCI backend: prepares bundles from the image store and delegates the
/// isolation boundary to a Boxcar-compatible provider.
pub struct OciBackend {
    store: ImageStore,
}

impl OciBackend {
    /// Creates the backend over an image store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            store: ImageStore::open(root)?,
        })
    }

    /// Access to the underlying image store.
    pub fn store(&self) -> &ImageStore {
        &self.store
    }

    /// Resolves a workload's image reference to a validated bundle.
    pub fn prepare_bundle(&self, spec: &WorkloadSpec) -> Result<Bundle> {
        let image = match &spec.execution {
            tpt_runtime_model::execution::ExecutionSpec::OciImage(image) => image.clone(),
            other => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("oci backend cannot execute {other:?}"),
                ))
            }
        };
        let reference = ImageReference::parse(&image.image).map_err(|err| {
            RuntimeError::new(ErrorKind::InvalidConfiguration, err)
        })?;
        let mut bundle = self.store.resolve(&reference)?;
        bundle.args = {
            let mut args = Vec::new();
            match bundle_validate_entry(&bundle) {
                Some(entry) => args.push(entry),
                None => args.push("/bin/sh".to_owned()),
            }
            args.extend(image.args.iter().cloned());
            args
        };
        for (key, value) in &image.env {
            bundle.env.insert(key.clone(), value.clone());
        }
        bundle.validate()?;
        Ok(bundle)
    }
}

fn bundle_validate_entry(bundle: &Bundle) -> Option<String> {
    bundle.args.first().cloned()
}

impl ExecutionBackend for OciBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Oci
    }

    /// Preparation resolves and validates the bundle — everything short of
    /// the isolation boundary.
    fn prepare(&self, spec: &WorkloadSpec, _ctx: &StartContext) -> Result<()> {
        self.prepare_bundle(spec).map(|_| ())
    }

    /// Starting an OCI workload requires a process/isolation provider
    /// (Boxcar primitive). Until the `tpt-boxcar` integration lands this
    /// fails explicitly — never silently (SPEC §48).
    fn start(&self, _spec: &WorkloadSpec, _ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>> {
        Err(RuntimeError::new(
            ErrorKind::NotImplemented,
            "OCI workload start requires tpt-boxcar isolation primitives (Phase 4)",
        )
        .with_backend("oci")
        .with_operation("start"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_is_oci() {
        let base = std::env::temp_dir().join(format!("tpt-oci-b-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        assert_eq!(backend.kind(), tpt_runtime_model::BackendKind::Oci);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn start_fails_explicitly_pending_boxcar() {
        let base = std::env::temp_dir().join(format!("tpt-oci-c-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        let spec = WorkloadSpec::new(
            "db",
            tpt_runtime_model::execution::ExecutionSpec::OciImage(
                tpt_runtime_model::execution::OciImageSpec {
                    image: "postgres:16".to_owned(),
                    ..Default::default()
                },
            ),
        );
        let ctx = StartContext {
            workload_id: tpt_runtime_core::id::WorkloadId::generate(),
            mounts: vec![],
            log_dir: base.clone(),
            capabilities: tpt_runtime_capability::CapabilitySet::empty(),
            network_mode: tpt_runtime_model::network::NetworkMode::None,
            exposed_ports: vec![],
        };
        let err = match backend.start(&spec, &ctx) {
            Err(err) => err,
            Ok(_) => panic!("oci start must fail pending boxcar"),
        };
        assert_eq!(err.kind, ErrorKind::NotImplemented);
        assert_eq!(err.backend.as_deref(), Some("oci"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn prepare_reports_missing_image_clearly() {
        let base = std::env::temp_dir().join(format!("tpt-oci-d-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        let spec = WorkloadSpec::new(
            "db",
            tpt_runtime_model::execution::ExecutionSpec::OciImage(
                tpt_runtime_model::execution::OciImageSpec {
                    image: "postgres:16".to_owned(),
                    ..Default::default()
                },
            ),
        );
        let ctx = StartContext {
            workload_id: tpt_runtime_core::id::WorkloadId::generate(),
            mounts: vec![],
            log_dir: base.clone(),
            capabilities: tpt_runtime_capability::CapabilitySet::empty(),
            network_mode: tpt_runtime_model::network::NetworkMode::None,
            exposed_ports: vec![],
        };
        let err = backend.prepare(&spec, &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("tpt-boxcar"));
        std::fs::remove_dir_all(&base).ok();
    }
}
