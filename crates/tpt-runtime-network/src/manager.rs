//! Network assignment: turning intents into concrete allocations (SPEC §18).

use serde::{Deserialize, Serialize};
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::id::NetworkId;
use tpt_runtime_model::network::NetworkSpec;

/// One exposed inbound port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortAllocation {
    /// Logical name from the manifest (`http`).
    pub name: String,
    /// Host port the service is reachable on.
    pub host_port: u16,
    /// Whether the port was fixed by the manifest or picked at runtime.
    pub static_port: bool,
}

/// The concrete network realization for a workload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkAssignment {
    /// Network instance id.
    pub id: NetworkId,
    /// The intent this assignment fulfills.
    pub mode: tpt_runtime_model::network::NetworkMode,
    /// Allocated inbound ports (empty unless inbound is allowed).
    pub ports: Vec<PortAllocation>,
}

/// Allocates logical networks and host ports.
pub struct NetworkManager {
    /// Ports already handed out to running workloads.
    allocated: std::sync::Mutex<std::collections::BTreeSet<u16>>,
}

impl Default for NetworkManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkManager {
    /// Creates an empty manager.
    pub fn new() -> Self {
        Self {
            allocated: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    /// Resolves a workload's network request into a concrete assignment.
    ///
    /// Rejects `expose` entries for modes that cannot accept inbound
    /// traffic (SPEC §48: explicit failure, not silent degradation) and
    /// refuses statically requested ports that are already in use.
    pub async fn assign(&self, workload: &str, spec: &NetworkSpec) -> Result<NetworkAssignment> {
        let mut ports = Vec::new();
        if spec.mode.allows_inbound() {
            for (name, requested) in &spec.expose {
                let (host_port, static_port) = if *requested == 0 {
                    (self.pick_free_port().await, false)
                } else {
                    (*requested, true)
                };
                self.claim(workload, host_port, static_port)?;
                ports.push(PortAllocation {
                    name: name.clone(),
                    host_port,
                    static_port,
                });
            }
            ports.sort_by(|a, b| a.name.cmp(&b.name));
        } else if !spec.expose.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!(
                    "network mode '{}' cannot expose ports",
                    spec.mode
                ),
            )
            .with_operation("network.assign"));
        }

        Ok(NetworkAssignment {
            id: NetworkId::generate(),
            mode: spec.mode,
            ports,
        })
    }

    /// Releases ports held by an assignment when the workload stops.
    pub fn release(&self, assignment: &NetworkAssignment) {
        let mut allocated = self.allocated.lock().unwrap();
        for port in &assignment.ports {
            allocated.remove(&port.host_port);
        }
    }

    fn claim(&self, workload: &str, port: u16, static_port: bool) -> Result<()> {
        let mut allocated = self.allocated.lock().unwrap();
        if !static_port {
            // dynamically picked ports are free by construction
            allocated.insert(port);
            return Ok(());
        }
        if allocated.contains(&port) {
            return Err(RuntimeError::new(
                ErrorKind::NetworkFailure,
                format!("host port {port} requested by workload '{workload}' is already in use"),
            )
            .with_operation("network.assign"));
        }
        allocated.insert(port);
        Ok(())
    }

    async fn pick_free_port(&self) -> u16 {
        // Try a few ephemeral binds; the OS guarantees liveness of the pick
        // until the listener drops, which happens before the workload binds.
        for _ in 0..32 {
            if let Ok(listener) =
                tokio::net::TcpListener::bind(("127.0.0.1", 0)).await
            {
                if let Ok(addr) = listener.local_addr() {
                    drop(listener);
                    let port = addr.port();
                    let mut allocated = self.allocated.lock().unwrap();
                    if allocated.insert(port) {
                        return port;
                    }
                } else {
                    drop(listener);
                }
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec(mode: tpt_runtime_model::network::NetworkMode, expose: &[(&str, u16)]) -> NetworkSpec {
        NetworkSpec {
            mode,
            expose: expose
                .iter()
                .map(|(n, p)| (n.to_string(), *p))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[tokio::test]
    async fn none_mode_has_no_ports() {
        let manager = NetworkManager::new();
        let assignment = manager.assign("w", &spec(tpt_runtime_model::network::NetworkMode::None, &[])).await.unwrap();
        assert_eq!(assignment.mode, tpt_runtime_model::network::NetworkMode::None);
        assert!(assignment.ports.is_empty());
    }

    #[tokio::test]
    async fn service_mode_allocates_requested_and_dynamic_ports() {
        let manager = NetworkManager::new();
        let assignment = manager
            .assign(
                "api",
                &spec(
                    tpt_runtime_model::network::NetworkMode::Service,
                    &[("http", 8080), ("metrics", 0)],
                ),
            )
            .await
            .unwrap();
        assert_eq!(assignment.ports.len(), 2);
        let http = assignment.ports.iter().find(|p| p.name == "http").unwrap();
        assert_eq!(http.host_port, 8080);
        assert!(http.static_port);
        let metrics = assignment.ports.iter().find(|p| p.name == "metrics").unwrap();
        assert_ne!(metrics.host_port, 0);
        assert!(!metrics.static_port);

        // release frees the static port for the next workload
        manager.release(&assignment);
        let again = manager
            .assign("api2", &spec(tpt_runtime_model::network::NetworkMode::Service, &[("http", 8080)]))
            .await
            .unwrap();
        assert_eq!(again.ports[0].host_port, 8080);
    }

    #[tokio::test]
    async fn duplicate_static_port_fails_explicitly() {
        let manager = NetworkManager::new();
        let first = manager
            .assign("a", &spec(tpt_runtime_model::network::NetworkMode::Service, &[("http", 9000)]))
            .await
            .unwrap();
        let err = manager
            .assign("b", &spec(tpt_runtime_model::network::NetworkMode::Service, &[("http", 9000)]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NetworkFailure);
        manager.release(&first);
    }

    #[tokio::test]
    async fn expose_without_inbound_mode_fails() {
        let manager = NetworkManager::new();
        let err = manager
            .assign("a", &spec(tpt_runtime_model::network::NetworkMode::Outbound, &[("http", 80)]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NetworkFailure);
    }
}
