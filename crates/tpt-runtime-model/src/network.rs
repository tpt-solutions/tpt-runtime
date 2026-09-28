//! Logical network abstraction (SPEC §18).
//!
//! Workloads request an intent (`none`, `host`, `private`, `service`,
//! `isolated`, `outbound`); the runtime resolves the underlying mechanism.
//! Users never configure adapters, NAT or bridges for ordinary workloads.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Network intent requested by a workload (SPEC §18).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// No network access at all.
    #[default]
    None,
    /// Full access to the host network stack.
    Host,
    /// Workload-private network, isolated from other workloads.
    Private,
    /// Outbound connections allowed, no inbound.
    Outbound,
    /// Reachable as a named service by other workloads.
    Service,
    /// Fully isolated (no traffic in or out).
    Isolated,
}

impl NetworkMode {
    /// Whether this mode permits outbound connections.
    pub fn allows_outbound(self) -> bool {
        matches!(
            self,
            NetworkMode::Host | NetworkMode::Outbound | NetworkMode::Service
        )
    }

    /// Whether this mode permits inbound listeners to be exposed.
    pub fn allows_inbound(self) -> bool {
        matches!(self, NetworkMode::Host | NetworkMode::Service)
    }
}

impl std::str::FromStr for NetworkMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "none" => Ok(NetworkMode::None),
            "host" => Ok(NetworkMode::Host),
            "private" => Ok(NetworkMode::Private),
            "outbound" => Ok(NetworkMode::Outbound),
            "service" => Ok(NetworkMode::Service),
            "isolated" => Ok(NetworkMode::Isolated),
            other => Err(format!(
                "unknown network mode '{other}' (expected none, host, private, outbound, service or isolated)"
            )),
        }
    }
}

impl fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl NetworkMode {
    fn as_str(&self) -> &'static str {
        match self {
            NetworkMode::None => "none",
            NetworkMode::Host => "host",
            NetworkMode::Private => "private",
            NetworkMode::Outbound => "outbound",
            NetworkMode::Service => "service",
            NetworkMode::Isolated => "isolated",
        }
    }
}

/// Network request of a workload (SPEC §18, §32 `[network]`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSpec {
    /// Network intent; defaults to `none` (deny by default, SPEC §5.2).
    #[serde(default)]
    pub mode: NetworkMode,
    /// Named inbound ports to expose (`http = 8080`); only meaningful when
    /// the mode allows inbound.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub expose: BTreeMap<String, u16>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_by_default() {
        assert_eq!(NetworkSpec::default().mode, NetworkMode::None);
        assert!(!NetworkMode::None.allows_outbound());
        assert!(!NetworkMode::Isolated.allows_outbound());
        assert!(NetworkMode::Outbound.allows_outbound());
    }

    #[test]
    fn inbound_only_for_exposing_modes() {
        assert!(NetworkMode::Host.allows_inbound());
        assert!(NetworkMode::Service.allows_inbound());
        assert!(!NetworkMode::Outbound.allows_inbound());
    }

    #[test]
    fn parses_all_spec_modes() {
        for text in ["none", "host", "private", "service", "isolated", "outbound"] {
            assert!(text.parse::<NetworkMode>().is_ok());
        }
        assert!("bridged".parse::<NetworkMode>().is_err());
    }

    #[test]
    fn expose_map_serializes() {
        let spec = NetworkSpec {
            mode: NetworkMode::Service,
            expose: [("http".to_owned(), 8080)].into_iter().collect(),
        };
        let value: serde_json::Value = serde_json::to_value(&spec).unwrap();
        assert_eq!(value["expose"]["http"], 8080);
    }
}
