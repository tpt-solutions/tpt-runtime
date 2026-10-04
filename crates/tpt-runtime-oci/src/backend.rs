//! OCI execution backend on Boxcar-compatible primitives (SPEC §13, §17).

use std::path::PathBuf;
use std::sync::Arc;
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

/// The isolation boundary: what actually runs a prepared bundle.
///
/// Declared here, not in the implementing crate, so `tpt-runtime-oci` never
/// depends on a provider. The runtime's own Windows implementation lives in
/// `tpt-runtime-sandbox`; Boxcar could implement this same trait later.
pub trait IsolationProvider: Send + Sync {
    /// Starts `bundle` as an isolated workload and returns its handle.
    ///
    /// `spec` supplies the resource ceilings the provider should enforce, so
    /// no provider needs to re-resolve policy.
    fn start(
        &self,
        bundle: &Bundle,
        spec: &WorkloadSpec,
        ctx: &StartContext,
    ) -> Result<Box<dyn WorkloadInstance>>;
}

/// OCI backend: pulls and prepares bundles from the image store and
/// delegates the isolation boundary to a [`IsolationProvider`].
pub struct OciBackend {
    store: ImageStore,
    pull_policy: PullPolicy,
    max_blob_bytes: u64,
    provider: Option<Arc<dyn IsolationProvider>>,
}

impl OciBackend {
    /// Creates the backend over an image store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            store: ImageStore::open(root)?,
            pull_policy: PullPolicy::default(),
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            provider: None,
        })
    }

    /// Installs the isolation provider that runs prepared bundles.
    ///
    /// Without one the backend still resolves and pulls images, but `start`
    /// reports `not_implemented` — it never silently succeeds (SPEC §48).
    pub fn with_provider(mut self, provider: Arc<dyn IsolationProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Whether an isolation provider is installed.
    pub fn has_provider(&self) -> bool {
        self.provider.is_some()
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

    /// Hands the prepared bundle to the isolation provider.
    ///
    /// With no provider installed this fails explicitly with
    /// `not_implemented` rather than pretending to have started something
    /// (SPEC §48).
    fn start(&self, spec: &WorkloadSpec, ctx: &StartContext) -> Result<Box<dyn WorkloadInstance>> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::NotImplemented,
                "OCI workload start requires an isolation provider; \
                 install one with OciBackend::with_provider",
            )
            .with_backend("oci")
            .with_operation("start")
        })?;

        let bundle = self
            .prepare_bundle(spec)
            .map_err(|err| err.with_backend("oci").with_operation("start"))?;
        provider
            .start(&bundle, spec, ctx)
            .map_err(|err| err.with_backend("oci").with_workload(spec.name.clone()))
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
    fn start_fails_explicitly_without_a_provider() {
        let base = std::env::temp_dir().join(format!("tpt-oci-c-{}", std::process::id()));
        let backend = OciBackend::new(&base).unwrap();
        assert!(!backend.has_provider());
        let ctx = ctx(&base);
        let err = match backend.start(&oci_spec("postgres:16"), &ctx) {
            Err(err) => err,
            Ok(_) => panic!("oci start must fail when no isolation provider is installed"),
        };
        assert_eq!(err.kind, ErrorKind::NotImplemented);
        assert_eq!(err.backend.as_deref(), Some("oci"));
        assert!(err.message.contains("with_provider"), "{err}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_provider_can_be_installed() {
        let base = std::env::temp_dir().join(format!("tpt-oci-f-{}", std::process::id()));
        let backend = OciBackend::new(&base)
            .unwrap()
            .with_provider(std::sync::Arc::new(StubProvider));
        assert!(backend.has_provider());
        std::fs::remove_dir_all(&base).ok();
    }

    /// A provider that always fails, used to prove the wiring is reachable.
    struct StubProvider;

    impl IsolationProvider for StubProvider {
        fn start(
            &self,
            _bundle: &Bundle,
            _spec: &WorkloadSpec,
            _ctx: &StartContext,
        ) -> Result<Box<dyn WorkloadInstance>> {
            Err(RuntimeError::new(
                ErrorKind::System,
                "stub provider reached",
            ))
        }
    }

    #[test]
    fn provider_errors_are_attributed_to_the_oci_backend() {
        let base = std::env::temp_dir().join(format!("tpt-oci-g-{}", std::process::id()));
        let backend = OciBackend::new(&base)
            .unwrap()
            .with_provider(std::sync::Arc::new(StubProvider));
        // The image is absent from the store, so prepare fails first; the
        // point is that a provider being installed does not mask that.
        let err = match backend.start(&oci_spec("postgres:16"), &ctx(&base)) {
            Err(err) => err,
            Ok(_) => panic!("expected a not-found error for an uncached image"),
        };
        assert_eq!(err.kind, ErrorKind::NotFound);
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
