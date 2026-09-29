//! End-to-end: workload manager → Windows process backend → capture,
//! accounting and events (SPEC §45 integration tests, windows row).

#[cfg(windows)]
mod windows_e2e {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tpt_runtime_core::event::EventKind;
    use tpt_runtime_core::state::WorkloadState;
    use tpt_runtime_device::DeviceRegistry;
    use tpt_runtime_model::execution::{ExecutionSpec, WindowsProcessSpec};
    use tpt_runtime_network::NetworkManager;
    use tpt_runtime_observe::{EventHub, MetricsRegistry};
    use tpt_runtime_policy::PolicyEngine;
    use tpt_runtime_security::SecretStore;
    use tpt_runtime_storage::StorageManager;
    use tpt_runtime_windows::WindowsProcessBackend;
    use tpt_runtime_workload::{LogsQuery, WorkloadManager};

    fn manager(tag: &str) -> (Arc<WorkloadManager>, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("tpt-e2e-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let events = Arc::new(EventHub::new(None::<std::path::PathBuf>));
        let metrics = Arc::new(MetricsRegistry::new());
        let storage = Arc::new(Mutex::new(
            StorageManager::open(base.join("volumes")).unwrap(),
        ));
        let manager = Arc::new(WorkloadManager::new(
            base.join("logs"),
            events,
            metrics,
            storage,
            Arc::new(NetworkManager::new()),
            Arc::new(Mutex::new(DeviceRegistry::new())),
            Arc::new(Mutex::new(
                SecretStore::open(base.join("secrets.json")).unwrap(),
            )),
            PolicyEngine::new(tpt_runtime_policy::HostCapacity::unknown()),
        ));
        manager.register_backend(Arc::new(WindowsProcessBackend::new()));
        (manager, base)
    }

    fn echo_spec(name: &str) -> tpt_runtime_model::workload::WorkloadSpec {
        tpt_runtime_model::workload::WorkloadSpec::new(
            name,
            ExecutionSpec::WindowsProcess(WindowsProcessSpec {
                program: "cmd.exe".to_owned(),
                args: vec!["/C".to_owned(), "echo integration-marker".to_owned()],
                ..Default::default()
            }),
        )
    }

    #[test]
    fn full_lifecycle_with_logs_and_events() {
        let (manager, base) = manager("full");
        let id = manager.create(echo_spec("echo-workload")).unwrap();

        // created, not yet started
        let info = manager.inspect("echo-workload").unwrap();
        assert_eq!(info.state, WorkloadState::Created);

        manager.start(&id).unwrap();
        // let the workload exit on its own, then verify a clean stop
        let status = manager.wait(&id, Duration::from_secs(15)).unwrap();
        assert_eq!(status.code, Some(0), "cmd /C echo must exit cleanly");

        let info = manager.inspect("echo-workload").unwrap();
        assert_eq!(info.state, WorkloadState::Stopped);
        assert_eq!(info.exit_code, Some(0));
        assert!(info.usage.is_some(), "final usage sample recorded");

        // stdout captured through the whole stack
        let lines = manager.logs(&id, LogsQuery::Stdout, 10).unwrap();
        assert!(
            lines.iter().any(|l| l.contains("integration-marker")),
            "logs: {lines:?}"
        );

        // event trail covers the lifecycle
        // (events went to the hub; the sink is absent so check live
        // emission by subscribing and running another workload)
        let mut sub = manager.events().subscribe();
        let id2 = manager.create(echo_spec("echo-second")).unwrap();
        manager.start(&id2).unwrap();
        let mut seen = Vec::new();
        for _ in 0..4 {
            if let Ok(event) = sub.try_recv() {
                seen.push(event.event);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(seen.contains(&EventKind::WorkloadStarted), "seen: {seen:?}");

        // destroy clears the record
        manager.wait(&id2, Duration::from_secs(15)).unwrap();
        manager.destroy(&id2).unwrap();
        assert!(manager.inspect("echo-second").is_err());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn duplicate_names_rejected_and_missing_not_found() {
        let (manager, base) = manager("names");
        manager.create(echo_spec("dup")).unwrap();
        assert!(manager.create(echo_spec("dup")).is_err());
        assert!(manager.inspect("no-such-workload").is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn volumes_resolve_into_mount_context() {
        let (manager, base) = manager("volumes");
        manager.storage().lock().unwrap().create("proj").unwrap();
        let mut spec = echo_spec("mounted");
        spec.volumes = vec![tpt_runtime_model::volume::VolumeMount {
            name: "proj".to_owned(),
            mount: "/workspace".to_owned(),
            mode: tpt_runtime_model::volume::VolumeAccessMode::ReadWrite,
        }];
        let id = manager.create(spec).unwrap();
        let info = manager.inspect("mounted").unwrap();
        assert_eq!(info.mounts.len(), 1);
        assert!(info.mounts[0].host_path.contains("proj"));
        // a never-started (Created) workload can be destroyed directly
        manager.destroy(&id).unwrap();
        std::fs::remove_dir_all(&base).ok();
    }
}
