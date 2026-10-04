//! OCI execution backend on Boxcar-compatible primitives (SPEC §13, §17).

use std::path::PathBuf;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::execution::oci_ref::ImageReference;
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_process::{ExecutionBackend, StartContext, WorkloadInstance};

use crate::bundle::Bundle;
use crate::store::ImageStore;

/// When the backend may reach a registry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullPolicy {
    /// Local store only: missing images fail with a clear error.
    #[default]
    Never,
    /// Pull from the registry when the image is not in the local store
    /// (layers already cached are not re-downloaded).
    IfMissing,
}

/// Cap applied to any single blob download (decompression-bomb guard).
pub const DEFAULT_MAX_BLOB_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// OCI backend: pulls and prepares bundles from the image store and
/// delegates the isolation boundary to a Boxcar-compatible provider.
pub struct OciBackend {
    store: ImageStore,
    pull_policy: PullPolicy,
    max_blob_bytes: u64,
}

impl OciBackend {
    /// Creates the backend over an image store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            store: ImageStore::open(root)?,
            pull_policy: PullPolicy::default(),
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
        })
    }

    /// Sets when the backend may pull from a registry.
    pub fn with_pull_policy(mut self, policy: PullPolicy) -> Self {
        self.pull_policy = policy;
        self
    }

    /// Overrides the per-blob download cap.
    pub fn with_max_blob_bytes(mut self, max: u64) -> Self {
        self.max_blob_bytes = max;
        self
    }

    /// The pull policy in effect.
    pub fn pull_policy(&self) -> PullPolicy {
        self.pull_policy
    }

    /// Access to the underlying image store.
    pub fn store(&self) -> &ImageStore {
        &self.store
    }

    /// Resolves a workload's image reference to a validated bundle,
    /// pulling from the registry when the policy allows and the image is
    /// missing locally.
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
        let reference = ImageReference::parse(&image.image)
            .map_err(|err| RuntimeError::new(ErrorKind::InvalidConfiguration, err))?;

        let mut bundle = match self.store.resolve(&reference) {
            Ok(bundle) => bundle,
            Err(err) => {
                if err.kind != ErrorKind::NotFound || self.pull_policy == PullPolicy::Never {
                    if err.kind == ErrorKind::NotFound {
                        return Err(RuntimeError::new(
                            ErrorKind::NotFound,
                            format!(
                                "image '{}' is not in the local store and pulling is disabled \
                                 (enable with pull policy 'if-missing')",
                                image.image
                            ),
                        )
                        .with_backend("oci")
                        .with_workload(spec.name.clone()));
                    }
                    return Err(err);
                }
                let client = if self.insecure_registry(&reference) {
                    crate::registry::RegistryClient::insecure(&reference)
                } else {
                    crate::registry::RegistryClient::for_reference(&reference)
                };
                crate::image::pull(&client, &self.store, &reference, self.max_blob_bytes)
                    .map_err(|err| err.with_workload(spec.name.clone()))?
            }
        };

        // The manifest's argv stays authoritative; user args append.
        bundle.args.extend(image.args.iter().cloned());
        for (key, value) in &image.env {
            bundle.env.insert(key.clone(), value.clone());
        }
        bundle.validate()?;
        Ok(bundle)
    }

    /// Plain-HTTP registries: loopback hosts only (tests, local dev).
    fn insecure_registry(&self, reference: &ImageReference) -> bool {
        let (host, _) = crate::registry::RegistryClient::split_registry(&reference.repository);
        let bare = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(&host);
        bare == "localhost" || bare.starts_with("127.") || host == "[::1]" || host == "::1"
    }
}

impl ExecutionBackend for OciBackend {
    fn kind(&self) -> tpt_runtime_model::BackendKind {
        tpt_runtime_model::BackendKind::Oci
    }

    /// Preparation resolves (and, when allowed, pulls) and validates the
    /// bundle — everything short of the isolation boundary.
    fn prepare(&self, spec: &WorkloadSpec, _ctx: &StartContext) -> Result<()> {
        self.prepare_bundle(spec).map(|_| ())
    }

    /// Starting an OCI workload requires a process/isolation provider
    /// (Boxcar primitive). Until the `tpt-boxcar` integration lands this
    /// fails explicitly — never silently (SPEC §48).
    fn start(
        &self,
        _spec: &WorkloadSpec,
        _ctx: &StartContext,
    ) -> Result<Box<dyn WorkloadInstance>> {
        Err(RuntimeError::new(
            ErrorKind::NotImplemented,
            "OCI workload start requires an isolation provider (tpt-boxcar / Origin primitives)",
        )
        .with_backend("oci")
        .with_operation("start"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oci_spec(image: &str) -> WorkloadSpec {
        WorkloadSpec::new(
            "db",
            tpt_runtime_model::execution::ExecutionSpec::OciImage(
                tpt_runtime_model::execution::OciImageSpec {
                    image: image.to_owned(),
                    ..Default::default()
                },
            ),
        )
    }

    fn ctx(base: &std::path::Path) -> StartContext {
        StartContext {
            workload_id: tpt_runtime_core::id::WorkloadId::generate(),
            mounts: vec![],
            log_dir: base.to_path_buf(),
            capabilities: tpt_runtime_capability::CapabilitySet::empty(),
            network_mode: tpt_runtime_model::network::NetworkMode::None,
            exposed_ports: vec![],
        }
    }

    #[test]
    fn backend_kind_is_oci() {
        let base = std::env::temp_dir().join(format!("tpt-oci-b-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        assert_eq!(backend.kind(), tpt_runtime_model::BackendKind::Oci);
        assert_eq!(backend.pull_policy(), PullPolicy::Never);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn start_fails_explicitly_pending_isolation_provider() {
        let base = std::env::temp_dir().join(format!("tpt-oci-c-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        let ctx = ctx(&base);
        let err = match backend.start(&oci_spec("postgres:16"), &ctx) {
            Err(err) => err,
            Ok(_) => panic!("oci start must fail pending an isolation provider"),
        };
        assert_eq!(err.kind, ErrorKind::NotImplemented);
        assert_eq!(err.backend.as_deref(), Some("oci"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn prepare_reports_missing_image_clearly() {
        let base = std::env::temp_dir().join(format!("tpt-oci-d-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        let ctx = ctx(&base);
        let err = backend.prepare(&oci_spec("postgres:16"), &ctx).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("not in the local store"), "{err}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn loopback_registries_are_plain_http() {
        let base = std::env::temp_dir().join(format!("tpt-oci-e-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        let reference =
            tpt_runtime_model::execution::oci_ref::ImageReference::parse("localhost:5000/app:v1")
                .unwrap();
        assert!(backend.insecure_registry(&reference));
        let remote =
            tpt_runtime_model::execution::oci_ref::ImageReference::parse("postgres:16").unwrap();
        assert!(!backend.insecure_registry(&remote));
        std::fs::remove_dir_all(&base).ok();
    }
}
