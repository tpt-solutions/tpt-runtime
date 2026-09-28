//! # tpt-runtime-daemon
//!
//! The host daemon (SPEC §43 "Windows host daemon"): builds every
//! subsystem — storage, network, devices, secrets, events, metrics, the
//! backends — and serves the local API until shutdown.
//!
//! Process lifetime is tied to workloads: native workloads run in Job
//! Objects with kill-on-close, so a daemon crash cannot leave orphaned
//! workload processes behind (SPEC §12; SPEC §45 runtime-restart failure
//! behavior).

use std::sync::{Arc, Mutex};
use tpt_runtime_api::server::{serve, ApiState};
use tpt_runtime_config::DaemonConfig;
use tpt_runtime_core::error::Result;
use tpt_runtime_device::DeviceRegistry;
use tpt_runtime_network::NetworkManager;
use tpt_runtime_observe::{EventHub, MetricsRegistry};
use tpt_runtime_policy::PolicyEngine;
use tpt_runtime_security::SecretStore;
use tpt_runtime_storage::StorageManager;
use tpt_runtime_workload::WorkloadManager;

/// Builds the full runtime stack and serves until asked to stop.
pub async fn run(config: DaemonConfig) -> Result<()> {
    config.prepare_dirs()?;

    let events = Arc::new(EventHub::new(Some(config.events_file())));
    let metrics = Arc::new(MetricsRegistry::new());
    let storage = Arc::new(Mutex::new(StorageManager::open(config.volumes_dir())?));
    let network = Arc::new(NetworkManager::new());
    let devices = Arc::new(Mutex::new(DeviceRegistry::new()));
    let secrets = Arc::new(Mutex::new(SecretStore::open(config.secrets_file())?));

    // GPU discovery (SPEC §20): absent NVIDIA stack is not an error.
    match tpt_runtime_gpu::discover_gpus() {
        Ok(gpus) if gpus.is_empty() => {
            eprintln!("[daemon] no GPUs discovered");
        }
        Ok(gpus) => {
            let mut registry = devices.lock().unwrap();
            for gpu in gpus {
                eprintln!("[daemon] discovered gpu:{}: {}", gpu.index, gpu.name);
                registry.register(gpu.as_device());
            }
        }
        Err(err) => {
            eprintln!("[daemon] GPU discovery failed: {err}");
        }
    }

    let manager = Arc::new(WorkloadManager::new(
        config.logs_dir(),
        events.clone(),
        metrics.clone(),
        storage.clone(),
        network,
        devices,
        secrets,
        PolicyEngine::new(tpt_runtime_policy::HostCapacity::unknown()),
    ));

    // Backends (SPEC §10). OCI and Linux register so their prepare paths
    // give precise errors; their start paths report pending integrations.
    manager.register_backend(Arc::new(tpt_runtime_windows::WindowsProcessBackend::new()));
    manager.register_backend(Arc::new(tpt_runtime_wasm::WasmBackend::new()?));
    manager.register_backend(Arc::new(tpt_runtime_oci::OciBackend::new(
        config.state_dir.join("images"),
    )?));
    manager.register_backend(Arc::new(tpt_runtime_linux::LinuxBackend::new()));

    eprintln!(
        "[daemon] tpt-runtime {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        config.pipe_name
    );

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let state = Arc::new(ApiState {
        manager,
        started_at: std::time::Instant::now(),
        shutdown: shutdown_tx,
    });

    // Ctrl+C triggers the same shutdown path as daemon.shutdown.
    let ctrl_c_state = state.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("[daemon] shutdown requested (ctrl+c)");
            let _ = ctrl_c_state.shutdown.send(true);
        }
    });

    let _ = shutdown_rx;
    serve(config.clone(), state.clone()).await?;

    // Graceful shutdown: stop whatever is still running. If the daemon is
    // killed hard instead, job kill-on-close reaps the workloads.
    for info in state.manager.list() {
        if info.state.is_active() {
            eprintln!("[daemon] stopping workload '{}' ({})", info.name, info.id);
            let _ = state
                .manager
                .stop_and_wait(&info.id, std::time::Duration::from_secs(5));
        }
    }

    eprintln!("[daemon] stopped");
    Ok(())
}
