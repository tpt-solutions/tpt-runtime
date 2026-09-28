//! Secret storage and capability-gated resolution (SPEC §24).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tpt_runtime_capability::Capability;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::timestamp::Timestamp;

/// A secret as persisted on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SecretRecord {
    value: String,
    created_at: Timestamp,
}

/// Listing entry: names only, never values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SecretSummary {
    /// Secret name.
    pub name: String,
    /// Creation time.
    pub created_at: Timestamp,
}

/// File-backed secret store.
///
/// Values are stored at `<path>` as JSON. The MVP stores plaintext at rest
/// under the user profile with no network exposure; see
/// `docs/ARCHITECTURE.md` §"Secrets" for the DPAPI/TPM roadmap. The store
/// enforces one rule unconditionally: values require a capability.
pub struct SecretStore {
    path: PathBuf,
    secrets: BTreeMap<String, SecretRecord>,
}

impl SecretStore {
    /// Opens (or creates) a store at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let secrets = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str(&raw).map_err(|err| {
                RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("corrupt secret store {}: {err}", path.display()),
                )
            })?
        } else {
            BTreeMap::new()
        };
        Ok(Self { path, secrets })
    }

    /// Creates or replaces a secret. Existing values are overwritten
    /// (`tpt secret create` semantics).
    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        let name = validate_name(name)?;
        if value.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "secret value must not be empty",
            ));
        }
        self.secrets.insert(
            name.to_owned(),
            SecretRecord {
                value: value.to_owned(),
                created_at: Timestamp::now(),
            },
        );
        self.persist()
    }

    /// Deletes a secret. Returns whether it existed.
    pub fn delete(&mut self, name: &str) -> Result<bool> {
        let name = validate_name(name)?;
        let existed = self.secrets.remove(name).is_some();
        if existed {
            self.persist()?;
        }
        Ok(existed)
    }

    /// Lists secret names with metadata (no values).
    pub fn list(&self) -> Vec<SecretSummary> {
        self.secrets
            .iter()
            .map(|(name, record)| SecretSummary {
                name: name.clone(),
                created_at: record.created_at,
            })
            .collect()
    }

    /// Resolves a secret **only with its capability** (SPEC §24: a workload
    /// receives an ephemeral capability, not the value by default).
    pub fn resolve(&self, capability: &Capability) -> Result<String> {
        let name = match capability {
            Capability::Secret { name } => name.as_str(),
            other => {
                return Err(RuntimeError::new(
                    ErrorKind::CapabilityDenied,
                    format!("secret resolution requires a secret capability, got '{}'", other.name()),
                ))
            }
        };
        self.secrets
            .get(name)
            .map(|record| record.value.clone())
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::NotFound,
                    format!("secret '{name}' does not exist"),
                )
            })
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(&self.secrets)?;
        // write-then-rename to avoid torn files
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, raw)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(name)
    } else {
        Err(RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("invalid secret name '{name}' (use 1..=64 chars of [A-Za-z0-9._-])"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        std::env::temp_dir()
            .join("tpt-secrets")
            .join(format!("{}-{}", std::process::id(), uuid::Uuid::new_v4().simple()))
    }

    #[test]
    fn set_list_delete_round_trip() {
        let path = temp_path();
        let mut store = SecretStore::open(&path).unwrap();
        store.set("github-token", "ghp_secret").unwrap();

        // persisted across reopen
        let store = SecretStore::open(&path).unwrap();
        let names = store.list();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name, "github-token");

        let mut store = SecretStore::open(&path).unwrap();
        assert!(store.delete("github-token").unwrap());
        assert!(!store.delete("github-token").unwrap());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolution_requires_the_capability() {
        let path = temp_path();
        let mut store = SecretStore::open(&path).unwrap();
        store.set("github-token", "ghp_secret").unwrap();

        // with the matching capability: works
        let cap = Capability::Secret {
            name: "github-token".to_owned(),
        };
        assert_eq!(store.resolve(&cap).unwrap(), "ghp_secret");

        // wrong capability kind: denied
        assert!(store.resolve(&Capability::NetworkOutbound).is_err());

        // unknown secret: not found
        let cap = Capability::Secret {
            name: "nope".to_owned(),
        };
        assert_eq!(store.resolve(&cap).unwrap_err().kind, ErrorKind::NotFound);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn listing_never_contains_values() {
        let path = temp_path();
        let mut store = SecretStore::open(&path).unwrap();
        store.set("api-key", "super-secret-value").unwrap();
        let listing = serde_json::to_string(&store.list()).unwrap();
        assert!(!listing.contains("super-secret-value"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn bad_names_rejected() {
        let path = temp_path();
        let mut store = SecretStore::open(&path).unwrap();
        assert!(store.set("../evil", "x").is_err());
        assert!(store.set("", "x").is_err());
        assert!(store.set("k", "").is_err());
    }
}
