//! Content-addressed image store (SPEC §13: image resolution + storage).
//!
//! Layout under the store root:
//!
//! ```text
//! images/
//! ├── refs/<repository-encoded>/<tag>   → manifest digest (text)
//! ├── blobs/sha256/<digest>             → raw blob files (manifests,
//! │                                       configs, layers), content-addressed
//! └── bundles/<digest>                  → unpacked bundle directories
//!     ├── rootfs/                       → assembled layer stack
//!     ├── config.json                   → OCI runtime-spec process config
//!     └── image.json                    → the image config's run defaults
//! ```
//!
//! Registry *pulling* (the network protocol side) lives in
//! [`crate::registry`]; this store is the content-addressed cache both
//! sides share (and the part `tpt-boxcar` primitives can adopt as-is).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::execution::oci_ref::ImageReference;

use crate::bundle::Bundle;

/// Run defaults recorded alongside an unpacked bundle (from the image
/// config blob): how a workload derived from this image starts.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ImageDefaults {
    /// Entrypoint argv from the image config.
    #[serde(default)]
    pub entrypoint: Vec<String>,
    /// Default arguments from the image config.
    #[serde(default)]
    pub cmd: Vec<String>,
    /// Environment as `KEY=VALUE` strings.
    #[serde(default)]
    pub env: Vec<String>,
    /// Working directory inside the rootfs.
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// Resolves image references to prepared bundles on local disk.
pub struct ImageStore {
    root: PathBuf,
}

impl ImageStore {
    /// Opens (and creates) an image store rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("blobs").join("sha256"))?;
        std::fs::create_dir_all(root.join("refs"))?;
        std::fs::create_dir_all(root.join("bundles"))?;
        Ok(Self { root })
    }

    /// The store root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stores raw bytes content-addressed; returns the digest. Idempotent:
    /// re-storing identical content is a no-op.
    pub fn store_blob(&self, bytes: &[u8]) -> Result<String> {
        let digest = crate::registry::digest_of(bytes);
        let dest = self.blob_path(&digest)?;
        if !dest.exists() {
            let tmp = dest.with_extension("part");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, &dest)?;
        }
        Ok(digest)
    }

    /// Whether a raw blob is present.
    pub fn has_blob(&self, digest: &str) -> bool {
        self.blob_path(digest).map(|p| p.is_file()).unwrap_or(false)
    }

    /// The path of a raw blob, when the digest is well-formed. Accepts the
    /// prefixed and bare-hex forms (the store pins bare hex).
    pub fn blob_path(&self, digest: &str) -> Result<PathBuf> {
        Ok(self
            .root
            .join("blobs")
            .join("sha256")
            .join(validate_stored_digest(digest)?))
    }

    /// Where an unpacked bundle for `digest` lives (prefixed or bare hex).
    pub fn bundle_dir(&self, digest: &str) -> Result<PathBuf> {
        Ok(self
            .root
            .join("bundles")
            .join(validate_stored_digest(digest)?))
    }

    /// Registers an already-unpacked bundle directory under `digest`,
    /// returning the canonical bundle path. The directory is adopted in
    /// place (no copy) when it already lives under the store.
    pub fn register_digest(&self, digest: &str, bundle_dir: &Path) -> Result<PathBuf> {
        let key = validate_digest(digest)?;
        if !bundle_dir.join("rootfs").is_dir() {
            return Err(RuntimeError::new(
                ErrorKind::StorageFailure,
                format!(
                    "bundle at '{}' has no rootfs; refusing to register",
                    bundle_dir.display()
                ),
            ));
        }
        let dest = self.bundle_dir(&key)?;
        if dest == bundle_dir {
            return Ok(dest);
        }
        std::fs::rename(bundle_dir, &dest).or_else(|_| {
            if dest.is_dir() {
                Ok(())
            } else {
                Err(RuntimeError::new(
                    ErrorKind::StorageFailure,
                    format!(
                        "cannot move bundle into store ({} → {})",
                        bundle_dir.display(),
                        dest.display()
                    ),
                ))
            }
        })?;
        Ok(dest)
    }

    /// Pins a tag to a digest (`refs/<repo>/<tag>` file).
    pub fn tag(&self, reference: &ImageReference, digest: &str) -> Result<()> {
        let key = validate_digest(digest)?;
        let ref_dir = self
            .root
            .join("refs")
            .join(encode_repo(&reference.repository));
        std::fs::create_dir_all(&ref_dir)?;
        let tag = reference.tag.clone().unwrap_or_else(|| "latest".to_owned());
        std::fs::write(ref_dir.join(tag), &key)?;
        Ok(())
    }

    /// The manifest digest a reference resolves to, if pinned locally.
    pub fn manifest_digest(&self, reference: &ImageReference) -> Result<String> {
        let tag = reference.tag.clone().unwrap_or_else(|| "latest".to_owned());
        let ref_file = self
            .root
            .join("refs")
            .join(encode_repo(&reference.repository))
            .join(&tag);
        let digest = std::fs::read_to_string(&ref_file).map_err(|_| {
            RuntimeError::new(
                ErrorKind::NotFound,
                format!(
                    "image '{}:{}' is not in the local store; pull it (or enable pulling) first",
                    reference.repository, tag
                ),
            )
        })?;
        validate_stored_digest(&digest)
    }

    /// Resolves a reference to its bundle, when the image is present
    /// locally (pulled, or seeded via [`ImageStore::register_digest`]).
    pub fn resolve(&self, reference: &ImageReference) -> Result<Bundle> {
        let digest = self.manifest_digest(reference)?;
        let bundle_path = self.bundle_dir(&digest)?;
        if !bundle_path.join("rootfs").is_dir() {
            return Err(RuntimeError::new(
                ErrorKind::NotFound,
                format!(
                    "image '{}:{}' is pinned but not unpacked; pull it first",
                    reference.repository,
                    reference.tag.clone().unwrap_or_else(|| "latest".to_owned()),
                ),
            ));
        }

        let defaults: ImageDefaults = serde_json::from_str(
            &std::fs::read_to_string(bundle_path.join("image.json")).unwrap_or_default(),
        )
        .unwrap_or_default();

        let mut env = BTreeMap::new();
        for entry in &defaults.env {
            if let Some((key, value)) = entry.split_once('=') {
                env.insert(key.to_owned(), value.to_owned());
            }
        }

        let mut args = defaults.entrypoint.clone();
        args.extend(defaults.cmd.clone());
        if args.is_empty() {
            args.push("/bin/sh".to_owned());
        }

        Ok(Bundle {
            id: digest[..16].to_owned(),
            path: bundle_path,
            args,
            env,
            working_dir: defaults.working_dir.filter(|dir| !dir.is_empty()),
        })
    }
}

fn validate_digest(digest: &str) -> Result<String> {
    let digest = digest.trim();
    let hex = digest.strip_prefix("sha256:").ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("unsupported digest '{digest}' (want sha256:...)"),
        )
    })?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("malformed sha256 digest '{digest}'"),
        ));
    }
    Ok(hex.to_ascii_lowercase())
}

/// Validates a digest as read back from a ref file: the bare hex written
/// by [`ImageStore::tag`] and the full `sha256:...` form are both legal;
/// anything else (including path traversal) is rejected.
fn validate_stored_digest(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let hex = raw.strip_prefix("sha256:").unwrap_or(raw);
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("malformed digest in image ref: '{raw}'"),
        ));
    }
    Ok(hex.to_ascii_lowercase())
}

/// Filesystem-safe encoding of a repository path (keeps slashes).
fn encode_repo(repository: &str) -> String {
    repository
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn store() -> (ImageStore, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "tpt-oci-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        (ImageStore::open(&base).unwrap(), base)
    }

    #[test]
    fn blobs_are_content_addressed_and_idempotent() {
        let (store, base) = store();
        let digest = store.store_blob(b"layer-bytes").unwrap();
        assert!(digest.starts_with("sha256:"));
        assert!(store.has_blob(&digest));
        // re-store is a no-op, same content-addressed path
        assert_eq!(store.store_blob(b"layer-bytes").unwrap(), digest);
        assert_eq!(
            store.blob_path(&digest).unwrap().metadata().unwrap().len(),
            11
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn tag_and_resolve_round_trip() {
        let (store, base) = store();
        let bundle_dir = base.join("incoming");
        std::fs::create_dir_all(bundle_dir.join("rootfs")).unwrap();
        std::fs::write(
            bundle_dir.join("image.json"),
            r#"{"entrypoint": ["bin/app"], "cmd": ["--serve"], "env": ["K=V"]}"#,
        )
        .unwrap();
        store.register_digest(DIGEST, &bundle_dir).unwrap();

        let reference = ImageReference::parse("postgres:16").unwrap();
        store.tag(&reference, DIGEST).unwrap();

        let bundle = store.resolve(&reference).unwrap();
        assert!(bundle.rootfs_path().is_dir());
        assert_eq!(bundle.args, ["bin/app", "--serve"]);
        assert_eq!(bundle.env.get("K").map(String::as_str), Some("V"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn unknown_image_reports_local_miss() {
        let (store, base) = store();
        let reference = ImageReference::parse("postgres:16").unwrap();
        let err = store.resolve(&reference).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("not in the local store"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn malformed_digests_rejected() {
        let (store, base) = store();
        let dir = base.join("x");
        std::fs::create_dir_all(dir.join("rootfs")).unwrap();
        assert!(store.register_digest("md5:deadbeef", &dir).is_err());
        assert!(store.register_digest("sha256:short", &dir).is_err());
        assert!(!store.has_blob("garbage"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn register_refuses_bundle_without_rootfs() {
        let (store, base) = store();
        let dir = base.join("x");
        std::fs::create_dir_all(&dir).unwrap();
        let err = store.register_digest(DIGEST, &dir).unwrap_err();
        assert_eq!(err.kind, ErrorKind::StorageFailure);
        std::fs::remove_dir_all(&base).ok();
    }
}
