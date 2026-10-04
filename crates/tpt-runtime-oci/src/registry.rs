//! Docker Registry HTTP API v2 client (SPEC §13).
//!
//! Pulls manifests, image configs and layers with full digest verification
//! (sha256 over every byte that enters the content store). Authentication
//! follows the standard anonymous token flow (`401` + `WWW-Authenticate:
//! Bearer` → token endpoint → `Authorization: Bearer`); private registries
//! fail loudly instead of silently degrading (SPEC §48).
//!
//! Boxcar-compatible: this client is the pull primitive the runtime owns
//! until (and after) `tpt-boxcar` grows one; isolation itself remains the
//! provider's job.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::execution::oci_ref::ImageReference;

/// Media types accepted for manifests, most specific first.
pub const ACCEPT_MANIFEST: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

/// The default registry API host for references without one.
pub const DEFAULT_REGISTRY_HOST: &str = "registry-1.docker.io";

/// A client for one registry host.
pub struct RegistryClient {
    host: String,
    insecure_http: bool,
    agent: ureq::Agent,
    tokens: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl RegistryClient {
    /// Splits a reference's repository into registry host and remote name,
    /// applying Docker Hub conventions (official images live under
    /// `library/`, the API lives on `registry-1.docker.io`).
    pub fn split_registry(repository: &str) -> (String, String) {
        let first = repository.split('/').next().unwrap_or(repository);
        let has_host = first.contains('.') || first.contains(':') || first == "localhost";
        if has_host {
            (
                first.to_owned(),
                repository
                    .trim_start_matches(first)
                    .trim_start_matches('/')
                    .to_owned(),
            )
        } else if repository.contains('/') {
            (DEFAULT_REGISTRY_HOST.to_owned(), repository.to_owned())
        } else {
            (
                DEFAULT_REGISTRY_HOST.to_owned(),
                format!("library/{repository}"),
            )
        }
    }

    /// Creates a client for the registry implied by `reference` over HTTPS.
    pub fn for_reference(reference: &ImageReference) -> Self {
        let (host, _) = Self::split_registry(&reference.repository);
        Self::with_host(host)
    }

    /// Plain-HTTP client (localhost registries, tests).
    pub fn insecure(reference: &ImageReference) -> Self {
        let mut client = Self::for_reference(reference);
        client.insecure_http = true;
        client
    }

    fn with_host(host: String) -> Self {
        // Status codes are handled here (auth flow, 404 attribution), so
        // the agent must not turn them into transport errors.
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build()
            .into();
        Self {
            host,
            insecure_http: false,
            agent,
            tokens: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// The registry host this client talks to.
    pub fn host(&self) -> &str {
        &self.host
    }

    fn scheme(&self) -> &'static str {
        if self.insecure_http {
            "http"
        } else {
            "https"
        }
    }

    fn get(&self, url: &str, accept: Option<&str>) -> Result<ureq::http::Response<ureq::Body>> {
        let mut request = self.agent.get(url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        let response = request.call().map_err(|err| {
            RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("registry request to {url} failed: {err}"),
            )
            .with_backend("oci")
        })?;
        Ok(response)
    }

    /// Performs a GET, handling the anonymous Bearer token flow and
    /// retrying once with the acquired token.
    fn get_authorized(
        &self,
        path: &str,
        repo: &str,
        accept: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let url = format!("{}://{host}/v2/{path}", self.scheme(), host = self.host);
        let response = self.get(&url, accept)?;
        if response.status().as_u16() != 401 {
            return Ok(response);
        }

        let challenge = response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .unwrap_or_default();
        if !challenge.starts_with("Bearer") {
            return Err(RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!(
                    "registry '{host}' requires authentication ({challenge}); anonymous pull of '{repo}' refused",
                    host = self.host
                ),
            )
            .with_backend("oci"));
        }
        let token = self.acquire_token(&challenge)?;

        let mut authorized = ureq::get(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Accept", accept.unwrap_or("*/*"));
        let _ = &mut authorized;
        let retried = authorized.call().map_err(|err| {
            RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("authenticated registry request to {url} failed: {err}"),
            )
            .with_backend("oci")
        })?;
        Ok(retried)
    }

    /// Exchanges a `WWW-Authenticate: Bearer realm=...` challenge for a
    /// token, caching per scope.
    fn acquire_token(&self, challenge: &str) -> Result<String> {
        let realm = param_of(challenge, "realm").ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("bearer challenge without realm: '{challenge}'"),
            )
            .with_backend("oci")
        })?;
        let service = param_of(challenge, "service");
        let scope = param_of(challenge, "scope");

        let cache_key = format!(
            "{}|{}",
            service.as_deref().unwrap_or(""),
            scope.as_deref().unwrap_or("")
        );
        if let Some(token) = self.tokens.lock().unwrap().get(&cache_key) {
            return Ok(token.clone());
        }

        let mut url = realm.clone();
        let mut query = Vec::new();
        if let Some(service) = &service {
            query.push(format!("service={}", urlencode(service)));
        }
        if let Some(scope) = &scope {
            query.push(format!("scope={}", urlencode(scope)));
        }
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query.join("&"));
        }

        let mut response = self.get(&url, Some("application/json"))?;
        let body = response.body_mut().read_to_vec().map_err(|err| {
            RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("token endpoint {realm} failed: {err}"),
            )
            .with_backend("oci")
        })?;
        let parsed: serde_json::Value = serde_json::from_slice(&body).map_err(|err| {
            RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("token endpoint {realm} returned non-JSON: {err}"),
            )
            .with_backend("oci")
        })?;
        let token = parsed["token"]
            .as_str()
            .or_else(|| parsed["access_token"].as_str())
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::NetworkFailure,
                    format!("token endpoint {realm} returned no token"),
                )
                .with_backend("oci")
            })?
            .to_owned();
        self.tokens.lock().unwrap().insert(cache_key, token.clone());
        Ok(token)
    }

    /// Fetches a manifest (or index) by tag or digest. Indexes are resolved
    /// to the `linux/amd64` (then any linux) image manifest. When the
    /// reference pins a digest, the served bytes must hash to it.
    pub fn pull_manifest(&self, repo: &str, reference: &ImageReference) -> Result<PulledManifest> {
        let locator = reference
            .digest
            .clone()
            .unwrap_or_else(|| reference.tag.clone().unwrap_or_else(|| "latest".to_owned()));
        let path = format!("{repo}/manifests/{locator}");
        let mut response = self.get_authorized(&path, repo, Some(ACCEPT_MANIFEST))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(RuntimeError::new(
                if status == 404 {
                    ErrorKind::NotFound
                } else {
                    ErrorKind::NetworkFailure
                },
                format!("manifest '{repo}:{locator}' not resolvable (HTTP {status})"),
            )
            .with_backend("oci"));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = response.body_mut().read_to_vec().map_err(|err| {
            RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("manifest download failed: {err}"),
            )
            .with_backend("oci")
        })?;

        if let Some(digest) = &reference.digest {
            verify_bytes(&bytes, digest)?;
        }

        if content_type.contains("index") || content_type.contains("list") {
            let index: ImageIndex = serde_json::from_slice(&bytes).map_err(bad_manifest)?;
            let selected = index
                .manifests
                .iter()
                .find(|d| {
                    d.platform
                        .as_ref()
                        .map(|p| p.os == "linux" && p.architecture == "amd64")
                        .unwrap_or(false)
                })
                .or_else(|| {
                    index.manifests.iter().find(|d| {
                        d.platform
                            .as_ref()
                            .map(|p| p.os == "linux")
                            .unwrap_or(false)
                    })
                })
                .ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::NotFound,
                        format!("index for '{repo}:{locator}' has no linux image"),
                    )
                    .with_backend("oci")
                })?;
            let child = ImageReference {
                repository: reference.repository.clone(),
                tag: None,
                digest: Some(selected.digest.clone()),
            };
            return self.pull_manifest(repo, &child);
        }

        let manifest: ImageManifest = serde_json::from_slice(&bytes).map_err(bad_manifest)?;
        let digest = digest_of(&bytes);
        Ok(PulledManifest {
            bytes,
            digest,
            manifest,
        })
    }

    /// Streams a blob (config or layer) to `dest`, hashing while reading,
    /// then verifies the registry digest and size cap and renames into
    /// place. A byte-flipped or truncated layer never enters the store.
    pub fn fetch_blob(&self, repo: &str, digest: &str, dest: &Path, max_bytes: u64) -> Result<()> {
        if !digest.starts_with("sha256:") {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("unsupported blob digest '{digest}' (want sha256:...)"),
            )
            .with_backend("oci"));
        }
        let path = format!("{repo}/blobs/{digest}");
        let response = self.get_authorized(&path, repo, None)?;
        if response.status().as_u16() != 200 {
            return Err(RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!(
                    "blob '{digest}' fetch failed (HTTP {})",
                    response.status().as_u16()
                ),
            )
            .with_backend("oci"));
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = dest.with_extension("part");
        let mut file = std::fs::File::create(&tmp)?;
        let mut hasher = Sha256::new();
        let mut reader = response.into_body().into_reader();
        let mut buf = [0u8; 64 * 1024];
        let mut total: u64 = 0;
        loop {
            let read = reader.read(&mut buf)?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > max_bytes {
                let _ = std::fs::remove_file(&tmp);
                return Err(RuntimeError::new(
                    ErrorKind::StorageFailure,
                    format!("blob '{digest}' exceeds the {max_bytes} byte cap; refusing (decompression-bomb guard)"),
                )
                .with_backend("oci"));
            }
            hasher.update(&buf[..read]);
            std::io::Write::write_all(&mut file, &buf[..read])?;
        }
        std::io::Write::flush(&mut file)?;
        drop(file);

        let actual = format!("sha256:{}", hex(&hasher.finalize()));
        if actual != digest.to_ascii_lowercase() {
            let _ = std::fs::remove_file(&tmp);
            return Err(RuntimeError::new(
                ErrorKind::StorageFailure,
                format!("blob digest mismatch: registry said {digest}, bytes hash to {actual}"),
            )
            .with_backend("oci"));
        }
        std::fs::rename(&tmp, dest)?;
        Ok(())
    }
}

/// A manifest as pulled from the registry, plus its content digest.
pub struct PulledManifest {
    /// Raw manifest bytes (stored verbatim, content-addressed).
    pub bytes: Vec<u8>,
    /// sha256 of [`PulledManifest::bytes`].
    pub digest: String,
    /// The parsed image manifest.
    pub manifest: ImageManifest,
}

/// An OCI/Docker image manifest.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ImageManifest {
    /// Manifest schema version (2).
    #[serde(default)]
    pub schema_version: i64,
    /// The image config blob descriptor.
    pub config: BlobDescriptor,
    /// The layer descriptors, in application order.
    #[serde(default)]
    pub layers: Vec<BlobDescriptor>,
}

/// A content descriptor inside a manifest (registry JSON is camelCase).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobDescriptor {
    /// Blob media type (`application/vnd.oci.image.layer.v1.tar+gzip`).
    #[serde(default)]
    pub media_type: String,
    /// Content digest (`sha256:...`).
    pub digest: String,
    /// Blob size in bytes, when declared.
    #[serde(default)]
    pub size: i64,
}

/// A multi-platform image index (OCI) / manifest list (Docker).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ImageIndex {
    /// Platform-specific image manifests.
    #[serde(default)]
    pub manifests: Vec<IndexEntry>,
}

/// One entry of an image index.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct IndexEntry {
    /// Referenced manifest digest.
    pub digest: String,
    /// Target platform, when declared.
    #[serde(default)]
    pub platform: Option<Platform>,
}

/// The platform an image manifest targets.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Platform {
    /// OS (`linux`).
    pub os: String,
    /// Architecture (`amd64`).
    pub architecture: String,
}

/// The image config blob: run-time defaults for the containerized process.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ImageConfig {
    /// The container runtime configuration.
    pub config: ContainerConfig,
}

/// Process defaults from the image config. Real config blobs use
/// Docker's PascalCase keys.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerConfig {
    /// Entrypoint argv, executed inside the rootfs.
    #[serde(default)]
    pub entrypoint: Vec<String>,
    /// Default argument vector appended to the entrypoint.
    #[serde(default)]
    pub cmd: Vec<String>,
    /// Environment as `KEY=VALUE` strings.
    #[serde(default)]
    pub env: Vec<String>,
    /// Working directory inside the rootfs.
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl ImageConfig {
    /// Fetches and verifies the config blob described by a manifest.
    pub fn fetch(
        client: &RegistryClient,
        repo: &str,
        descriptor: &BlobDescriptor,
        tmp_dir: &Path,
        max_bytes: u64,
    ) -> Result<ImageConfig> {
        let dest: PathBuf = tmp_dir.join("config.blob");
        client.fetch_blob(repo, &descriptor.digest, &dest, max_bytes)?;
        let bytes = std::fs::read(&dest)?;
        verify_bytes(&bytes, &descriptor.digest)?;
        let _ = std::fs::remove_file(&dest);
        serde_json::from_slice(&bytes).map_err(|err| {
            RuntimeError::new(
                ErrorKind::StorageFailure,
                format!("image config blob is not a valid image config: {err}"),
            )
            .with_backend("oci")
        })
    }
}

/// Extracts a quoted parameter from a `WWW-Authenticate` challenge.
fn param_of(challenge: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = challenge.find(&needle)? + needle.len();
    let rest = &challenge[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// Minimal percent-encoding for token-endpoint queries.
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// sha256 of `bytes`, `sha256:`-prefixed, lowercase.
pub fn digest_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex(&hasher.finalize()))
}

/// Lowercase hex encoding.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verifies raw bytes against a `sha256:` digest.
pub fn verify_bytes(bytes: &[u8], digest: &str) -> Result<()> {
    let actual = digest_of(bytes);
    if actual != digest.to_ascii_lowercase() {
        return Err(RuntimeError::new(
            ErrorKind::StorageFailure,
            format!("content digest mismatch: expected {digest}, got {actual}"),
        )
        .with_backend("oci"));
    }
    Ok(())
}

fn bad_manifest(err: serde_json::Error) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::StorageFailure,
        format!("registry returned an unparsable manifest: {err}"),
    )
    .with_backend("oci")
}

/// Timeout applied to registry requests via the implicit agent defaults;
/// exported for tests that assert sane behavior without a server.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
