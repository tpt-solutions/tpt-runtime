//! Remote runtime transport (SPEC §39): the same NDJSON framing over TCP,
//! driven end to end against a live manager stack.

use std::sync::{Arc, Mutex};
use tpt_runtime_api::client::ApiClient;
use tpt_runtime_api::server::{serve_tcp_listener, ApiState};
use tpt_runtime_config::DaemonConfig;
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_device::DeviceRegistry;
use tpt_runtime_network::NetworkManager;
use tpt_runtime_observe::{EventHub, MetricsRegistry};
use tpt_runtime_policy::PolicyEngine;
use tpt_runtime_security::SecretStore;
use tpt_runtime_storage::StorageManager;
use tpt_runtime_windows::WindowsProcessBackend;
use tpt_runtime_workload::WorkloadManager;

#[tokio::test(flavor = "multi_thread")]
async fn remote_transport_creates_and_lists_workloads() {
    let base = std::env::temp_dir().join(format!("tpt-tcp-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let manager = Arc::new(WorkloadManager::new(
        base.join("logs"),
        Arc::new(EventHub::new(None::<std::path::PathBuf>)),
        Arc::new(MetricsRegistry::new()),
        Arc::new(Mutex::new(
            StorageManager::open(base.join("volumes")).unwrap(),
        )),
        Arc::new(NetworkManager::new()),
        Arc::new(Mutex::new(DeviceRegistry::new())),
        Arc::new(Mutex::new(
            SecretStore::open(base.join("secrets.json")).unwrap(),
        )),
        PolicyEngine::new(tpt_runtime_policy::HostCapacity::unknown()),
    ));
    manager.register_backend(Arc::new(WindowsProcessBackend::new()));

    let state = Arc::new(ApiState {
        manager: manager.clone(),
        started_at: std::time::Instant::now(),
        shutdown: tokio::sync::watch::channel(false).0,
        gpu: None,
    });

    // Bind port 0 and hand the listener to the server: the client learns
    // the port like any remote peer would.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let _ = serve_tcp_listener(listener, state).await;
    });

    let config = DaemonConfig {
        tcp: Some(format!("127.0.0.1:{port}")),
        ..DaemonConfig::default()
    };
    let mut client = ApiClient::connect(&config).await.unwrap();

    let status = client
        .call("daemon.status", serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(status["workloads"].as_u64(), Some(0));

    // A manifest round trip over the wire.
    let manifest = r#"
api = "tpt.runtime/v1"
[workload]
name = "remote-echo"
[execution]
backend = "windows"
program = "cmd.exe"
args = ["/C", "echo over-tcp"]
"#;
    let created = client
        .call(
            "workloads.create",
            serde_json::json!({ "manifest": manifest }),
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_owned();

    let list = client
        .call("workloads.list", serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"].as_str(), Some("remote-echo"));

    client
        .call("workloads.start", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    let info = client
        .call("workloads.inspect", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    assert_eq!(info["name"].as_str(), Some("remote-echo"));

    manager
        .stop_and_wait(
            &WorkloadId::from_raw(&id),
            std::time::Duration::from_secs(10),
        )
        .unwrap();
    client
        .call("workloads.destroy", serde_json::json!({ "id": id }))
        .await
        .unwrap();

    server.abort();
    std::fs::remove_dir_all(&base).ok();
}
