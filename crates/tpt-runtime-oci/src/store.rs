//! Content-addressed image store (SPEC §13: image resolution + storage).
//!
//! Layout under the store root:
//!
//! ```text
//! images/
//! ├── refs/<repository-encoded>/<tag>           → digest file
//! └── blobs/sha256/<digest>                     → unpacked bundle directory
//! ```
//!
//! Registry *pulling* (the network protocol side) is a Boxcar primitive and
//! intentionally absent: this store resolves references against blobs that
//! were seeded out-of-band (see `seed_from_directory`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::execution::oci_ref::ImageReference;

use crate::bundle::Bundle;

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
        Ok(Self { root })
    }

    /// The store root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Registers an already-unpacked bundle directory under `digest`,
    /// returning the canonical blob path. The directory is adopted in place
    /// (no copy) when it already lives under the store.
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
        let blob = self.root.join("blobs").join("sha256").join(&key);
        if blob == bundle_dir {
            return Ok(blob);
        }
        std::fs::rename(bundle_dir, &blob).or_else(|_| {
            if blob.is_dir() {
                Ok(())
            } else {
                Err(RuntimeError::new(
                    ErrorKind::StorageFailure,
                    format!(
                        "cannot move bundle into store ({} → {})",
                        bundle_dir.display(),
                        blob.display()
                    ),
                ))
            }
        })?;
        Ok(blob)
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

    /// Resolves a reference to its bundle, when the digest is present
    /// locally. Network resolution (Boxcar primitive) is not implemented.
    pub fn resolve(&self, reference: &ImageReference) -> Result<Bundle> {
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
                    "image '{}:{}' is not in the local store; pulling requires tpt-boxcar primitives",
                    reference.repository, tag
                ),
            )
        })?;
        // The ref file is untrusted input: validate before it touches a
        // path, so a poisoned tag cannot point outside the blob store.
        // Tags store the bare hex; accept the prefixed form too.
        let digest = validate_stored_digest(&digest)?;
        let blob = self.root.join("blobs").join("sha256").join(&digest);
        if !blob.join("rootfs").is_dir() {
            return Err(RuntimeError::new(
                ErrorKind::NotFound,
                format!(
                    "digest blob for '{}:{}' missing rootfs at '{}'",
                    reference.repository,
                    tag,
                    blob.display()
                ),
            ));
        }
        Ok(Bundle {
            id: digest.chars().take(16).collect(),
            path: blob,
            args: vec![],
            env: BTreeMap::new(),
            working_dir: None,
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
    fn tag_and_resolve_round_trip() {
        let (store, base) = store();
        let bundle_dir = base.join("incoming");
        std::fs::create_dir_all(bundle_dir.join("rootfs")).unwrap();
        store.register_digest(DIGEST, &bundle_dir).unwrap();

        let reference = ImageReference::parse("postgres:16").unwrap();
        store.tag(&reference, DIGEST).unwrap();

        let bundle = store.resolve(&reference).unwrap();
        assert!(bundle.rootfs_path().is_dir());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn unknown_image_names_boxcar_dependency() {
        let (store, base) = store();
        let reference = ImageReference::parse("postgres:16").unwrap();
        let err = store.resolve(&reference).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("tpt-boxcar"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn malformed_digests_rejected() {
        let (store, base) = store();
        let dir = base.join("x");
        std::fs::create_dir_all(dir.join("rootfs")).unwrap();
        assert!(store.register_digest("md5:deadbeef", &dir).is_err());
        assert!(store.register_digest("sha256:short", &dir).is_err());
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
