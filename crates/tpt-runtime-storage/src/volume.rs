//! Directory-backed logical volumes (SPEC §15).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::id::VolumeId;
use tpt_runtime_core::timestamp::Timestamp;

/// A logical volume: identity plus backing store (SPEC §15).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// Stable volume id.
    pub id: VolumeId,
    /// Logical name (`project`), unique within the runtime.
    pub name: String,
    /// Absolute host path backing the volume.
    pub backing_path: PathBuf,
    /// Creation time.
    pub created_at: Timestamp,
}

impl Volume {
    /// Whether a mount access mode may be served by this volume. The MVP
    /// storage layer serves both modes; read-only enforcement happens in
    /// backends that can enforce it (WASI preopens; Windows relies on
    /// policy, see `docs/ARCHITECTURE.md`).
    pub fn supports_mount(&self, mode: tpt_runtime_model::volume::VolumeAccessMode) -> bool {
        let _ = mode;
        true
    }
}

/// Summary of a volume for API/CLI listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VolumeInfo {
    /// Volume id.
    pub id: VolumeId,
    /// Logical name.
    pub name: String,
    /// Host path backing the volume.
    pub backing_path: String,
    /// Creation time.
    pub created_at: Timestamp,
}

impl From<&Volume> for VolumeInfo {
    fn from(volume: &Volume) -> Self {
        Self {
            id: volume.id.clone(),
            name: volume.name.clone(),
            backing_path: volume.backing_path.display().to_string(),
            created_at: volume.created_at,
        }
    }
}

/// Manages the runtime's logical volumes.
///
/// Volumes live under `<state>/volumes/<name>` and persist across daemon
/// restarts. Names follow the workload-name character set.
pub struct StorageManager {
    base_dir: PathBuf,
    volumes: BTreeMap<String, Volume>,
}

impl StorageManager {
    /// Creates a manager rooted at `base_dir` and loads existing volumes.
    pub fn open(base_dir: impl Into<PathBuf>) -> Result<Self> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        let mut manager = Self {
            base_dir,
            volumes: BTreeMap::new(),
        };
        manager.reload()?;
        Ok(manager)
    }

    /// Re-scans the volume directory, registering found volumes.
    pub fn reload(&mut self) -> Result<()> {
        self.volumes.clear();
        for entry in std::fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let meta_path = entry.path().join("volume.json");
            let volume = if meta_path.exists() {
                let raw = std::fs::read_to_string(&meta_path)?;
                serde_json::from_str::<Volume>(&raw).map_err(|err| {
                    RuntimeError::new(
                        ErrorKind::StorageFailure,
                        format!("corrupt volume metadata in '{}': {err}", entry.path().display()),
                    )
                })?
            } else {
                // A directory that appeared without metadata (e.g. restored
                // from backup): adopt it with fresh identity.
                Volume {
                    id: VolumeId::generate(),
                    name: name.clone(),
                    backing_path: entry.path(),
                    created_at: Timestamp::now(),
                }
            };
            self.volumes.insert(name, volume);
        }
        Ok(())
    }

    /// Creates a new logical volume.
    pub fn create(&mut self, name: &str) -> Result<Volume> {
        validate_volume_name(name)?;
        if self.volumes.contains_key(name) {
            return Err(RuntimeError::new(
                ErrorKind::StorageFailure,
                format!("volume '{name}' already exists"),
            ));
        }
        let backing_path = self.base_dir.join(name);
        std::fs::create_dir_all(&backing_path)?;
        let volume = Volume {
            id: VolumeId::generate(),
            name: name.to_owned(),
            backing_path,
            created_at: Timestamp::now(),
        };
        let meta = serde_json::to_string_pretty(&volume)?;
        std::fs::write(volume.backing_path.join("volume.json"), meta)?;
        self.volumes.insert(name.to_owned(), volume.clone());
        Ok(volume)
    }

    /// Resolves a logical volume by name.
    pub fn get(&self, name: &str) -> Result<&Volume> {
        self.volumes.get(name).ok_or_else(|| {
            RuntimeError::new(ErrorKind::NotFound, format!("volume '{name}' does not exist"))
        })
    }

    /// Lists all volumes.
    pub fn list(&self) -> Vec<VolumeInfo> {
        self.volumes.values().map(VolumeInfo::from).collect()
    }

    /// Removes a volume. Refuses non-empty volumes to avoid data loss.
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let volume = self.get(name)?;
        let path = volume.backing_path.clone();
        let entries: Vec<_> = std::fs::read_dir(&path)?
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name())
            .collect();
        let non_meta: Vec<_> = entries
            .iter()
            .filter(|n| n.as_os_str() != std::ffi::OsStr::new("volume.json"))
            .collect();
        if !non_meta.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::StorageFailure,
                format!(
                    "volume '{name}' is not empty ({}); remove its contents first",
                    non_meta.len()
                ),
            ));
        }
        std::fs::remove_dir_all(&path)?;
        self.volumes.remove(name);
        Ok(())
    }

    /// Host path for a mount of this volume (SPEC §15: multiple views).
    pub fn mount_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.get(name)?.backing_path.clone())
    }

    /// The directory root for all volumes.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

fn validate_volume_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && name != "."
        && name != "..";
    if ok {
        Ok(())
    } else {
        Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("invalid volume name '{name}' (use 1..=64 chars of [A-Za-z0-9._-])"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tpt-storage-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_get_list_remove_round_trip() {
        let base = temp_base();
        let mut manager = StorageManager::open(&base).unwrap();
        let volume = manager.create("project").unwrap();
        assert!(volume.backing_path.is_dir());
        assert_eq!(manager.get("project").unwrap().id, volume.id);
        assert_eq!(manager.list().len(), 1);

        // metadata survives reopen
        drop(manager);
        let manager = StorageManager::open(&base).unwrap();
        assert_eq!(manager.get("project").unwrap().id, volume.id);

        // non-empty refusal
        std::fs::write(volume.backing_path.join("data.txt"), "x").unwrap();
        let mut manager = StorageManager::open(&base).unwrap();
        let err = manager.remove("project").unwrap_err();
        assert_eq!(err.kind, ErrorKind::StorageFailure);

        // after removing the foreign content, removal works
        std::fs::remove_file(manager.get("project").unwrap().backing_path.join("data.txt")).unwrap();
        manager.remove("project").unwrap();
        assert!(manager.get("project").is_err());

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn removes_empty_volume() {
        let base = temp_base();
        let mut manager = StorageManager::open(&base).unwrap();
        manager.create("scratch").unwrap();
        manager.remove("scratch").unwrap();
        assert!(manager.get("scratch").is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn duplicate_names_rejected() {
        let base = temp_base();
        let mut manager = StorageManager::open(&base).unwrap();
        manager.create("v").unwrap();
        assert!(manager.create("v").is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn bad_names_rejected() {
        let base = temp_base();
        let mut manager = StorageManager::open(&base).unwrap();
        assert!(manager.create("../escape").is_err());
        assert!(manager.create("").is_err());
        assert!(manager.create("a b").is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn missing_volume_is_not_found() {
        let base = temp_base();
        let manager = StorageManager::open(&base).unwrap();
        let err = manager.get("nope").unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
