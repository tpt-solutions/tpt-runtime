//! Security isolation tests through the full manager stack (SPEC §46):
//! capability exactness, environment sanitization, secret non-leakage,
//! process/network/device isolation and loud storage failures.

#![cfg(windows)]

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tpt_runtime_capability::Capability;
use tpt_runtime_core::error::{ErrorKind, RuntimeError};
use tpt_runtime_core::event::{EventKind, RuntimeEvent};
use tpt_runtime_core::id::{DeviceId, WorkloadId};
use tpt_runtime_core::state::WorkloadState;
use tpt_runtime_device::{DeviceClass, DeviceInfo, DeviceRegistry};
use tpt_runtime_model::device::{DeviceAccessMode, DeviceRequest};
use tpt_runtime_model::execution::{ExecutionSpec, WindowsProcessSpec};
use tpt_runtime_model::network::{NetworkMode, NetworkSpec};
use tpt_runtime_model::resources::Memory;
use tpt_runtime_model::volume::{VolumeAccessMode, VolumeMount};
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_network::NetworkManager;
use tpt_runtime_observe::{EventHub, MetricsRegistry};
use tpt_runtime_policy::{HostCapacity, PolicyEngine};
use tpt_runtime_security::SecretStore;
use tpt_runtime_storage::StorageManager;
use tpt_runtime_windows::WindowsProcessBackend;
use tpt_runtime_workload::{LogsQuery, WorkloadManager};

fn build_manager(
    tag: &str,
    capacity: HostCapacity,
    register_backends: bool,
) -> (Arc<WorkloadManager>, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!("tpt-sec-{}-{}", tag, std::process::id()));
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
        PolicyEngine::new(capacity),
    ));
    if register_backends {
        manager.register_backend(Arc::new(WindowsProcessBackend::new()));
    }
    (manager, base)
}

fn manager_with_policy(
    tag: &str,
    capacity: HostCapacity,
) -> (Arc<WorkloadManager>, std::path::PathBuf) {
    build_manager(tag, capacity, true)
}

fn manager(tag: &str) -> (Arc<WorkloadManager>, std::path::PathBuf) {
    manager_with_policy(tag, HostCapacity::unknown())
}

/// A manager over an empty host: no backends registered.
fn bare_manager(tag: &str) -> (Arc<WorkloadManager>, std::path::PathBuf) {
    build_manager(tag, HostCapacity::unknown(), false)
}

fn cmd_spec(name: &str, args: &[&str]) -> WorkloadSpec {
    WorkloadSpec::new(
        name,
        ExecutionSpec::WindowsProcess(WindowsProcessSpec {
            program: "cmd.exe".to_owned(),
            args: std::iter::once("/C".to_owned())
                .chain(args.iter().map(|s| s.to_string()))
                .collect(),
            ..Default::default()
        }),
    )
}

fn echo_spec(name: &str) -> WorkloadSpec {
    cmd_spec(name, &["echo", &format!("marker-{name}")])
}

fn ping_spec(name: &str, ticks: u32) -> WorkloadSpec {
    WorkloadSpec::new(
        name,
        ExecutionSpec::WindowsProcess(WindowsProcessSpec {
            program: "ping.exe".to_owned(),
            args: vec!["-n".to_owned(), ticks.to_string(), "127.0.0.1".to_owned()],
            ..Default::default()
        }),
    )
}

fn drain(sub: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> Vec<RuntimeEvent> {
    let mut out = Vec::new();
    while let Ok(event) = sub.try_recv() {
        out.push(event);
    }
    out
}

fn workload_of(event: &RuntimeEvent, id: &WorkloadId) -> bool {
    event.workload.as_ref().map(|w| w.as_str()) == Some(id.as_str())
}

fn set_env(spec: &mut WorkloadSpec, key: &str, value: &str) {
    match &mut spec.execution {
        ExecutionSpec::WindowsProcess(p) => {
            p.env.insert(key.to_owned(), value.to_owned());
        }
        _ => unreachable!(),
    }
}

#[test]
fn child_environment_is_sanitized_not_inherited() {
    let (manager, base) = manager("env");
    // Probe with a host variable that is not on the allowlist.
    let probe = ["CARGO_MANIFEST_DIR", "APPDATA", "USERNAME", "COMPUTERNAME"]
        .into_iter()
        .find(|name| std::env::var_os(name).is_some());
    let Some(probe) = probe else { return };

    let mut spec = cmd_spec("env-probe", &["set"]);
    set_env(&mut spec, "WORKLOAD_MARKER", "isolation-ok");
    let id = manager.create(spec).unwrap();
    manager.start(&id).unwrap();
    let status = manager.wait(&id, Duration::from_secs(15)).unwrap();
    assert_eq!(status.code, Some(0));

    let stdout = manager
        .logs(&id, LogsQuery::Stdout, 2000)
        .unwrap()
        .join("\n");
    assert!(
        stdout.contains("WORKLOAD_MARKER=isolation-ok"),
        "manifest env must reach the child: {stdout:?}"
    );
    assert!(stdout.contains("SystemRoot="), "sanitized baseline present");
    assert!(
        !stdout.contains(&format!("{probe}=")),
        "host variable '{probe}' leaked into the workload environment"
    );
    manager.destroy(&id).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn secret_values_never_surface_on_any_manager_surface() {
    let (manager, base) = manager("secrets");
    manager
        .secrets()
        .lock()
        .unwrap()
        .set("db-password", "s3cr3t-value-42")
        .unwrap();

    let mut sub = manager.events().subscribe();

    let mut spec = echo_spec("with-secret");
    spec.capabilities = vec!["secret:db-password".to_owned()];
    let id = manager.create(spec).unwrap();

    // A capability naming a nonexistent secret fails create loudly.
    let mut bogus = echo_spec("bogus-secret");
    bogus.capabilities = vec!["secret:no-such-secret".to_owned()];
    assert_eq!(manager.create(bogus).unwrap_err().kind, ErrorKind::NotFound);

    manager.start(&id).unwrap();
    manager.wait(&id, Duration::from_secs(15)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let events = drain(&mut sub);

    let info = manager.inspect(id.as_str()).unwrap();
    assert!(info.capabilities.contains(&"secret:db-password".to_owned()));

    let surfaces = [
        serde_json::to_string(&info).unwrap(),
        serde_json::to_string(&manager.list()).unwrap(),
        serde_json::to_string(&events).unwrap(),
        manager
            .logs(&id, LogsQuery::Stdout, 100)
            .unwrap()
            .join("\n"),
        manager
            .logs(&id, LogsQuery::Stderr, 100)
            .unwrap_or_default()
            .join("\n"),
    ];
    for surface in &surfaces {
        assert!(
            !surface.contains("s3cr3t-value-42"),
            "secret value leaked through a manager surface: {surface}"
        );
    }

    // The value is released only against the matching capability.
    let store = manager.secrets().lock().unwrap();
    assert_eq!(
        store
            .resolve(&Capability::NetworkOutbound)
            .unwrap_err()
            .kind,
        ErrorKind::CapabilityDenied
    );
    drop(store);
    manager.destroy(&id).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn capability_grants_are_exact() {
    let (manager, base) = manager("caps");
    manager.storage().lock().unwrap().create("proj").unwrap();

    // A read-write mount must not silently add filesystem.write: grants
    // are exactly the names the manifest requested (SPEC §5.2).
    let mut spec = echo_spec("least-privilege");
    spec.capabilities = vec!["filesystem.read".to_owned()];
    spec.volumes = vec![VolumeMount {
        name: "proj".to_owned(),
        mount: "/workspace".to_owned(),
        mode: VolumeAccessMode::ReadWrite,
    }];
    let id = manager.create(spec).unwrap();
    let info = manager.inspect(id.as_str()).unwrap();
    assert_eq!(info.capabilities, vec!["filesystem.read".to_owned()]);
    manager.destroy(&id).unwrap();

    // Unknown grant names are carried for audit, never dropped silently.
    let mut spec = echo_spec("odd-grant");
    spec.capabilities = vec!["compiler.execute".to_owned()];
    let id = manager.create(spec).unwrap();
    assert_eq!(
        manager.inspect(id.as_str()).unwrap().capabilities,
        vec!["compiler.execute".to_owned()]
    );
    manager.destroy(&id).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn device_claims_conflict_and_release() {
    let (manager, base) = manager_with_policy(
        "devices",
        HostCapacity {
            cpu_cores: 8.0,
            memory: Memory::gib(16),
            gpus: 1,
        },
    );
    manager.devices().lock().unwrap().register(DeviceInfo {
        id: DeviceId::from_raw("gpu:0"),
        class: DeviceClass::Gpu,
        description: "test gpu".to_owned(),
        available: true,
    });

    // The policy layer denies indices beyond what was discovered.
    let mut ghost = echo_spec("ghost-gpu");
    ghost.devices = vec![DeviceRequest {
        id: "gpu:7".to_owned(),
        mode: DeviceAccessMode::Compute,
    }];
    assert_eq!(
        manager.create(ghost).unwrap_err().kind,
        ErrorKind::ResourceExhausted
    );

    let mut a = echo_spec("gpu-full");
    a.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Full,
    }];
    let id_a = manager.create(a).unwrap();
    manager.start(&id_a).unwrap();

    // A second exclusive claim while the first workload runs is denied.
    let mut b = echo_spec("gpu-full-2");
    b.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Full,
    }];
    assert_eq!(
        manager.create(b).unwrap_err().kind,
        ErrorKind::DeviceUnavailable
    );

    // Shared compute access alongside the exclusive holder is allowed.
    let mut c = echo_spec("gpu-compute");
    c.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Compute,
    }];
    let id_c = manager.create(c).unwrap();

    // When the workload settles, its claim must be released by the watcher.
    manager.wait(&id_a, Duration::from_secs(15)).unwrap();
    let claims = manager.devices().lock().unwrap().claims_of("gpu:0");
    assert!(
        !claims.contains(&"gpu-full".to_owned()),
        "claim of finished workload was not released: {claims:?}"
    );
    assert!(claims.contains(&"gpu-compute".to_owned()));

    manager.destroy(&id_a).unwrap();
    manager.destroy(&id_c).unwrap();
    assert!(manager
        .devices()
        .lock()
        .unwrap()
        .claims_of("gpu:0")
        .is_empty());

    // Exclusive attach works again once every claim is gone.
    let mut d = echo_spec("gpu-full-3");
    d.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Full,
    }];
    let id_d = manager.create(d).unwrap();
    manager.destroy(&id_d).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn device_disappearance_denies_new_attach() {
    let (manager, base) = manager_with_policy(
        "unplug",
        HostCapacity {
            cpu_cores: 8.0,
            memory: Memory::gib(16),
            gpus: 1,
        },
    );
    let gpu = |available: bool| DeviceInfo {
        id: DeviceId::from_raw("gpu:0"),
        class: DeviceClass::Gpu,
        description: "test gpu".to_owned(),
        available,
    };
    manager.devices().lock().unwrap().register(gpu(true));

    let mut a = echo_spec("holder");
    a.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Compute,
    }];
    let id_a = manager.create(a).unwrap();

    // The device disappears (driver crash, unplug): new attaches are
    // denied, existing claims are untouched.
    manager.devices().lock().unwrap().register(gpu(false));
    let mut b = echo_spec("late-attacher");
    b.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Compute,
    }];
    let err: RuntimeError = manager.create(b).unwrap_err();
    assert_eq!(err.kind, ErrorKind::DeviceUnavailable);
    assert!(err.message.contains("unavailable"));

    // Re-plugging restores attach.
    manager.devices().lock().unwrap().register(gpu(true));
    let mut c = echo_spec("replug");
    c.devices = vec![DeviceRequest {
        id: "gpu:0".to_owned(),
        mode: DeviceAccessMode::Compute,
    }];
    let id_c = manager.create(c).unwrap();

    manager.destroy(&id_a).unwrap();
    manager.destroy(&id_c).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn processes_are_isolated_between_workloads() {
    let (manager, base) = manager("isolation");
    let id_a = manager.create(ping_spec("iso-a", 8)).unwrap();
    let id_b = manager.create(ping_spec("iso-b", 8)).unwrap();
    manager.start(&id_a).unwrap();
    manager.start(&id_b).unwrap();

    assert_eq!(
        manager.inspect(id_a.as_str()).unwrap().state,
        WorkloadState::Running
    );

    // Stopping A must not touch B: each workload owns its job object.
    // (Job process counts wobble while the hidden console host attaches,
    // so B's isolation is proven by state and by its eventual clean exit.)
    let status = manager
        .stop_and_wait(&id_a, Duration::from_secs(10))
        .unwrap();
    assert!(status.killed);
    assert_eq!(
        manager.inspect(id_b.as_str()).unwrap().state,
        WorkloadState::Running
    );

    // B runs to its natural end: ping -n 8 cannot exit 0 if it had been
    // collateral damage of A's termination.
    let status_b = manager.wait(&id_b, Duration::from_secs(20)).unwrap();
    assert_eq!(status_b.code, Some(0), "B must exit cleanly, not killed");
    assert!(!status_b.killed);

    manager.destroy(&id_a).unwrap();
    manager.destroy(&id_b).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn network_is_denied_by_default_and_ports_cycle() {
    let (manager, base) = manager("network");
    let mut sub = manager.events().subscribe();

    // Deny-by-default: a None-mode workload gets no ports and no
    // network.connected event.
    let id_plain = manager.create(echo_spec("offline")).unwrap();
    manager.start(&id_plain).unwrap();
    manager.wait(&id_plain, Duration::from_secs(15)).unwrap();
    let info = manager.inspect(id_plain.as_str()).unwrap();
    assert_eq!(info.network_mode, "none");
    assert!(info.exposed_ports.is_empty());
    let events = drain(&mut sub);
    assert!(events
        .iter()
        .all(|e| !(e.event == EventKind::NetworkConnected && workload_of(e, &id_plain))));
    manager.destroy(&id_plain).unwrap();

    // Exposing ports requires an inbound-capable mode: rejected at create.
    let mut bad = echo_spec("bad-net");
    bad.network = NetworkSpec {
        mode: NetworkMode::Outbound,
        expose: [("http".to_owned(), 0)].into_iter().collect(),
    };
    assert_eq!(
        manager.create(bad).unwrap_err().kind,
        ErrorKind::InvalidConfiguration
    );

    // Service mode: dynamic allocation, conflict while live, release on stop.
    let mut a = ping_spec("svc-a", 2);
    a.network = NetworkSpec {
        mode: NetworkMode::Service,
        expose: [("http".to_owned(), 0)].into_iter().collect(),
    };
    let id_a = manager.create(a).unwrap();
    let port = *manager
        .inspect(id_a.as_str())
        .unwrap()
        .exposed_ports
        .get("http")
        .expect("service workload must expose its http port");
    assert_ne!(port, 0, "dynamic allocation must yield a real port");

    manager.start(&id_a).unwrap();
    let events = drain(&mut sub);
    assert!(events
        .iter()
        .any(|e| e.event == EventKind::NetworkConnected && workload_of(e, &id_a)));

    let mut conflict = ping_spec("svc-conflict", 30);
    conflict.network = NetworkSpec {
        mode: NetworkMode::Service,
        expose: [("http".to_owned(), port)].into_iter().collect(),
    };
    assert_eq!(
        manager.create(conflict).unwrap_err().kind,
        ErrorKind::NetworkFailure
    );

    // On settle the watcher releases the port; the same port binds again.
    manager.wait(&id_a, Duration::from_secs(15)).unwrap();
    let mut again = echo_spec("svc-again");
    again.network = NetworkSpec {
        mode: NetworkMode::Service,
        expose: [("http".to_owned(), port)].into_iter().collect(),
    };
    let id_again = manager.create(again).unwrap();
    manager.destroy(&id_again).unwrap();
    manager.destroy(&id_a).unwrap();
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn volume_failures_are_loud() {
    let (manager, base) = manager("storage");

    // Mounting a volume that does not exist fails at create.
    let mut ghost = echo_spec("ghost-volume");
    ghost.volumes = vec![VolumeMount {
        name: "ghost".to_owned(),
        mount: "/data".to_owned(),
        mode: VolumeAccessMode::ReadWrite,
    }];
    assert_eq!(manager.create(ghost).unwrap_err().kind, ErrorKind::NotFound);

    // A volume whose backing directory vanished fails prepare explicitly.
    manager.storage().lock().unwrap().create("gone").unwrap();
    let backing = manager
        .storage()
        .lock()
        .unwrap()
        .get("gone")
        .unwrap()
        .backing_path
        .clone();
    std::fs::remove_dir_all(&backing).unwrap();
    let mut gone = echo_spec("gone-volume");
    gone.volumes = vec![VolumeMount {
        name: "gone".to_owned(),
        mount: "/data".to_owned(),
        mode: VolumeAccessMode::ReadWrite,
    }];
    assert_eq!(
        manager.create(gone).unwrap_err().kind,
        ErrorKind::StorageFailure
    );
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn create_failure_names_the_missing_backend() {
    // No backends registered: create fails with attribution instead of
    // silently doing nothing.
    let (manager, base) = bare_manager("nobackend");
    let err = manager.create(echo_spec("orphan")).unwrap_err();
    assert_eq!(err.kind, ErrorKind::BackendUnavailable);
    assert!(err.message.contains("windows"));
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn overcommit_is_denied_not_clamped() {
    // Hard-limit policy: requesting more memory than the host has fails
    // admission (SPEC §25, §48).
    let (manager, base) = manager_with_policy(
        "overcommit",
        HostCapacity {
            cpu_cores: 2.0,
            memory: Memory::gib(1),
            gpus: 0,
        },
    );
    let mut spec = echo_spec("hog");
    spec.resources.memory = Some(Memory::gib(64));
    let err = manager.create(spec).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ResourceExhausted);
    assert!(err.message.contains("memory"));
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn btree_env_map_round_trips_through_manifest_env() {
    // Environment injection is manifest-scoped (BTreeMap ordering).
    let (manager, base) = manager("envmap");
    let mut spec = cmd_spec("envmap-probe", &["set"]);
    set_env(&mut spec, "ALPHA", "1");
    set_env(&mut spec, "BETA", "2");
    let id = manager.create(spec).unwrap();
    manager.start(&id).unwrap();
    manager.wait(&id, Duration::from_secs(15)).unwrap();
    let stdout = manager
        .logs(&id, LogsQuery::Stdout, 2000)
        .unwrap()
        .join("\n");
    assert!(stdout.contains("ALPHA=1"));
    assert!(stdout.contains("BETA=2"));
    manager.destroy(&id).unwrap();
    std::fs::remove_dir_all(&base).ok();
}
