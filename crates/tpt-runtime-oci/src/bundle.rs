//! OCI bundle model: what a prepared workload hands to an isolation
//! provider (Boxcar-compatible shape; OCI runtime-spec aligned fields).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// A prepared OCI bundle: rootfs plus process configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    /// Bundle id (equals the content digest prefix when unpacked from an
    /// image, or the workload id when synthesized).
    pub id: String,
    /// Absolute path to the bundle directory (contains `config.json` and
    /// `rootfs/` per the OCI runtime spec).
    pub path: PathBuf,
    /// Entry point args (`process.args[0]` is the program).
    pub args: Vec<String>,
    /// Environment for the containerized process.
    pub env: BTreeMap<String, String>,
    /// Working directory inside the rootfs.
    pub working_dir: Option<String>,
}

impl Bundle {
    /// Reads `config.json` from the bundle directory if present.
    pub fn config_path(&self) -> PathBuf {
        self.path.join("config.json")
    }

    /// The rootfs directory of this bundle.
    pub fn rootfs_path(&self) -> PathBuf {
        self.path.join("rootfs")
    }

    /// Verifies the bundle is structurally complete (rootfs exists).
    pub fn validate(&self) -> Result<()> {
        let rootfs = self.rootfs_path();
        if !rootfs.is_dir() {
            return Err(RuntimeError::new(
                ErrorKind::StorageFailure,
                format!("bundle '{}' has no rootfs directory", self.path.display()),
            ));
        }
        let entry = self.args.first().map(String::as_str).unwrap_or("");
        if entry.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("bundle '{}' has no entry point", self.path.display()),
            ));
        }
        // entry point must resolve inside the rootfs (no escaping views)
        let inside = sanitize_rootfs_entry(&rootfs, entry);
        if !inside {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!("bundle entry point '{entry}' escapes the rootfs"),
            ));
        }
        Ok(())
    }
}

/// Rootfs path check. A leading `/` is rootfs-relative by OCI convention
/// (the containerized process runs chrooted, so `/hello` means
/// `<rootfs>/hello`); what must never pass is `..` traversal above the
/// root, Windows-style paths (backslashes, drive-letter colons) and empty
/// entries.
fn sanitize_rootfs_entry(_rootfs: &Path, entry: &str) -> bool {
    let entry = entry.strip_prefix('/').unwrap_or(entry);
    if entry.is_empty() || entry.contains('\\') || entry.contains(':') {
        return false;
    }
    let mut depth: i64 = 0;
    for component in entry.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => depth += 1,
        }
    }
    depth > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_entry_outside_rootfs() {
        assert!(!sanitize_rootfs_entry(
            Path::new("/bundles/b"),
            "../../etc/passwd"
        ));
        assert!(!sanitize_rootfs_entry(
            Path::new("/bundles/b"),
            "a/../../.."
        ));
        assert!(!sanitize_rootfs_entry(Path::new("/bundles/b"), ""));
        assert!(!sanitize_rootfs_entry(Path::new("/bundles/b"), "/"));
        assert!(!sanitize_rootfs_entry(
            Path::new("/bundles/b"),
            "C:\\Windows\\System32\\cmd.exe"
        ));
        assert!(!sanitize_rootfs_entry(
            Path::new("/bundles/b"),
            "C:/Users/x/app"
        ));
        // Absolute entries are rootfs-relative by OCI convention.
        assert!(sanitize_rootfs_entry(Path::new("/bundles/b"), "bin/sh"));
        assert!(sanitize_rootfs_entry(
            Path::new("/bundles/b"),
            "usr/local/bin/app"
        ));
        assert!(sanitize_rootfs_entry(Path::new("/bundles/b"), "/bin/sh"));
        assert!(sanitize_rootfs_entry(Path::new("/bundles/b"), "/hello"));
    }

    #[test]
    fn validate_requires_rootfs_and_entry() {
        let base = std::env::temp_dir().join(format!("tpt-bundle-{}", std::process::id()));
        let bundle_path = base.join("b1");
        std::fs::create_dir_all(bundle_path.join("rootfs")).unwrap();
        let bundle = Bundle {
            id: "b1".to_owned(),
            path: bundle_path.clone(),
            args: vec!["bin/app".to_owned()],
            env: BTreeMap::new(),
            working_dir: None,
        };
        assert!(bundle.validate().is_ok());

        let no_entry = Bundle {
            args: vec![],
            id: "b2".to_owned(),
            path: bundle_path.clone(),
            env: BTreeMap::new(),
            working_dir: None,
        };
        assert!(no_entry.validate().is_err());

        let no_rootfs = Bundle {
            id: "b3".to_owned(),
            path: base.join("missing"),
            args: vec!["bin/app".to_owned()],
            env: BTreeMap::new(),
            working_dir: None,
        };
        assert!(no_rootfs.validate().is_err());
        std::fs::remove_dir_all(&base).ok();
    }
}
