//! Typed capabilities and the grant set (SPEC §5.2, §23).

use std::collections::BTreeSet;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};

/// A typed capability grant.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Read access to workload-visible volumes (`filesystem.read`).
    FilesystemRead,
    /// Write access to read-write mounted volumes (`filesystem.write`).
    FilesystemWrite,
    /// Outbound network connections (`network.outbound`).
    NetworkOutbound,
    /// Bind inbound listeners (`network.inbound`).
    NetworkInbound,
    /// Full host network access (`network.host`).
    NetworkHost,
    /// Access to a specific device (`device:<id>`).
    Device {
        /// Logical device id, e.g. `gpu:0`.
        id: String,
    },
    /// Access to a named secret (`secret:<name>`).
    Secret {
        /// Secret name, e.g. `github-token`.
        name: String,
    },
    /// Access to another workload's service endpoint (`ipc:service:<name>`).
    IpcService {
        /// Service name, e.g. `database`.
        service: String,
    },
    /// A recognized-but-untyped grant; carried for auditability.
    Other {
        /// Capability name as written in the manifest.
        name: String,
    },
}

impl serde::Serialize for Capability {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.name())
    }
}

impl<'de> serde::Deserialize<'de> for Capability {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Capability::parse(&name))
    }
}

impl Capability {
    /// Parses a manifest capability name into a typed capability.
    ///
    /// Accepted forms:
    ///
    /// - `filesystem.read`, `filesystem.write`
    /// - `network.outbound`, `network.inbound`, `network.host`
    /// - `device:<id>` — `device:gpu:0`
    /// - `secret:<name>` — `secret:github-token`
    /// - `ipc:service:<name>` — `ipc:service:database`
    /// - anything else becomes `Capability::Other` (auditable, unenforced)
    pub fn parse(name: &str) -> Capability {
        let name = name.trim();
        match name {
            "filesystem.read" => return Capability::FilesystemRead,
            "filesystem.write" => return Capability::FilesystemWrite,
            "network.outbound" => return Capability::NetworkOutbound,
            "network.inbound" => return Capability::NetworkInbound,
            "network.host" => return Capability::NetworkHost,
            _ => {}
        }
        if let Some(rest) = name.strip_prefix("device:") {
            return Capability::Device {
                id: rest.to_owned(),
            };
        }
        if let Some(rest) = name.strip_prefix("secret:") {
            return Capability::Secret {
                name: rest.to_owned(),
            };
        }
        if let Some(rest) = name.strip_prefix("ipc:service:") {
            return Capability::IpcService {
                service: rest.to_owned(),
            };
        }
        Capability::Other {
            name: name.to_owned(),
        }
    }

    /// The dotted/colon name of this capability as written in manifests.
    pub fn name(&self) -> String {
        match self {
            Capability::FilesystemRead => "filesystem.read".to_owned(),
            Capability::FilesystemWrite => "filesystem.write".to_owned(),
            Capability::NetworkOutbound => "network.outbound".to_owned(),
            Capability::NetworkInbound => "network.inbound".to_owned(),
            Capability::NetworkHost => "network.host".to_owned(),
            Capability::Device { id } => format!("device:{id}"),
            Capability::Secret { name } => format!("secret:{name}"),
            Capability::IpcService { service } => format!("ipc:service:{service}"),
            Capability::Other { name } => name.clone(),
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = RuntimeError;

    fn from_str(text: &str) -> Result<Self> {
        Ok(Capability::parse(text))
    }
}

/// The set of capabilities granted to one workload.
///
/// Grants are explicit: an empty set means the workload may not touch the
/// filesystem, the network, devices, secrets or other services.
/// Serialization is a plain array of capability names (audit-friendly).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    grants: BTreeSet<Capability>,
}

impl serde::Serialize for CapabilitySet {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.grants.iter())
    }
}

impl<'de> serde::Deserialize<'de> for CapabilitySet {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        let mut set = CapabilitySet::empty();
        for name in names {
            set.grant(Capability::parse(&name));
        }
        Ok(set)
    }
}

impl CapabilitySet {
    /// An empty set: no capabilities at all (deny by default, SPEC §5.2).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds a set from manifest capability names.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut set = Self::empty();
        for name in names {
            set.grant(Capability::parse(name));
        }
        set
    }

    /// Adds a grant.
    pub fn grant(&mut self, capability: Capability) {
        self.grants.insert(capability);
    }

    /// Removes a grant (revocation, SPEC §5.2). Returns whether it existed.
    pub fn revoke(&mut self, capability: &Capability) -> bool {
        self.grants.remove(capability)
    }

    /// All granted capabilities.
    pub fn grants(&self) -> impl Iterator<Item = &Capability> {
        self.grants.iter()
    }

    /// Number of grants.
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    /// Whether the set has no grants.
    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// Non-fatal check: is this capability granted?
    pub fn is_granted(&self, capability: &Capability) -> bool {
        self.grants.contains(capability)
    }

    /// Fatal check: returns `CapabilityDenied` when not granted, with full
    /// attribution (workload, operation).
    pub fn require(&self, capability: &Capability) -> Result<()> {
        if self.is_granted(capability) {
            Ok(())
        } else {
            Err(RuntimeError::new(
                ErrorKind::CapabilityDenied,
                format!(
                    "capability '{}' not granted to this workload",
                    capability.name()
                ),
            ))
        }
    }
}

/// The outcome of a [`CapabilitySet::require`] check, usable for
/// `capability.granted` / `capability.denied` events (SPEC §28).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityCheck {
    /// The capability that was checked.
    pub capability: Capability,
    /// Whether it was granted.
    pub granted: bool,
}

impl CapabilitySet {
    /// Performs a check and returns an event-ready [`CapabilityCheck`].
    pub fn check(&self, capability: Capability) -> CapabilityCheck {
        CapabilityCheck {
            granted: self.is_granted(&capability),
            capability,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_names() {
        assert_eq!(
            Capability::parse("filesystem.read"),
            Capability::FilesystemRead
        );
        assert_eq!(
            Capability::parse("network.outbound"),
            Capability::NetworkOutbound
        );
        assert_eq!(
            Capability::parse("device:gpu:0"),
            Capability::Device {
                id: "gpu:0".to_owned()
            }
        );
        assert_eq!(
            Capability::parse("secret:github-token"),
            Capability::Secret {
                name: "github-token".to_owned()
            }
        );
        assert_eq!(
            Capability::parse("ipc:service:database"),
            Capability::IpcService {
                service: "database".to_owned()
            }
        );
    }

    #[test]
    fn names_round_trip() {
        for name in [
            "filesystem.read",
            "filesystem.write",
            "network.host",
            "device:gpu:0",
            "secret:token",
            "ipc:service:db",
        ] {
            assert_eq!(Capability::parse(name).name(), name);
        }
    }

    #[test]
    fn unknown_names_are_other() {
        let cap = Capability::parse("compiler.execute");
        assert_eq!(
            cap,
            Capability::Other {
                name: "compiler.execute".to_owned()
            }
        );
        assert_eq!(cap.name(), "compiler.execute");
    }

    #[test]
    fn empty_set_denies_everything() {
        let set = CapabilitySet::empty();
        assert!(set.is_empty());
        assert!(set.require(&Capability::NetworkOutbound).is_err());
        assert!(!set.check(Capability::FilesystemWrite).granted);
    }

    #[test]
    fn grants_and_revocations() {
        let mut set = CapabilitySet::from_names(["network.outbound", "filesystem.read"]);
        assert_eq!(set.len(), 2);
        assert!(set.require(&Capability::NetworkOutbound).is_ok());
        assert!(set.revoke(&Capability::NetworkOutbound));
        assert!(set.require(&Capability::NetworkOutbound).is_err());
    }

    #[test]
    fn denial_error_mentions_capability_name() {
        let set = CapabilitySet::empty();
        let err = set
            .require(&Capability::parse("secret:github-token"))
            .unwrap_err();
        assert_eq!(
            err.kind,
            tpt_runtime_core::error::ErrorKind::CapabilityDenied
        );
        assert!(err.message.contains("secret:github-token"));
    }

    #[test]
    fn set_serializes_for_audit() {
        let set = CapabilitySet::from_names(["network.outbound"]);
        let value: serde_json::Value = serde_json::to_value(&set).unwrap();
        assert!(value.is_array());
        assert_eq!(value[0], "network.outbound");
    }
}
