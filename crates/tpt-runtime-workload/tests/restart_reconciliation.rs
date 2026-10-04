//! Restart reconciliation (SPEC Phase 9 snapshots): records persist across
//! manager generations — terminal records stay as history, mid-flight
//! workloads are attributed as `Failed (killed)` per kill-on-close, and
//! `Created` workloads stay startable.

#![cfg(windows)]

mod reconcile {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
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

    struct Env {
        manager: Arc<WorkloadManager>,
        base: std::path::PathBuf,
    }

    fn manager(tag: &str, snapshot: &std::path::Path) -> Env {
        let base =
            std::env::temp_dir().join(format!("tpt-reconcile-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let manager = Arc::new(
            WorkloadManager::new(
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
            )
            .with_snapshot_path(snapshot),
        );
        manager.register_backend(Arc::new(WindowsProcessBackend::new()));
        Env { manager, base }
    }

    fn cmd_spec(name: &str, args: &[&str]) -> tpt_runtime_model::workload::WorkloadSpec {
        tpt_runtime_model::workload::WorkloadSpec::new(
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

    fn ping_spec(name: &str) -> tpt_runtime_model::workload::WorkloadSpec {
        tpt_runtime_model::workload::WorkloadSpec::new(
            name,
            ExecutionSpec::WindowsProcess(WindowsProcessSpec {
                program: "ping.exe".to_owned(),
                args: vec!["-n".to_owned(), "30".to_owned(), "127.0.0.1".to_owned()],
                ..Default::default()
            }),
        )
    }

    #[test]
    fn records_survive_a_restart_and_mid_flight_is_attributed() {
        let snapshot =
            std::env::temp_dir().join(format!("tpt-reconcile-snap-{}.json", std::process::id()));

        // Generation 1: one finished workload, one created-only, one
        // mid-flight (running) — then the "daemon" disappears.
        let gen1 = manager("gen1", &snapshot);
        let echo = gen1
            .manager
            .create(cmd_spec("echo", &["echo", "reconcile-marker"]))
            .unwrap();
        gen1.manager.start(&echo).unwrap();
        gen1.manager.wait(&echo, Duration::from_secs(15)).unwrap();

        let created = gen1
            .manager
            .create(cmd_spec("created-only", &["echo", "never-ran"]))
            .unwrap();

        let running = gen1.manager.create(ping_spec("mid-flight")).unwrap();
        gen1.manager.start(&running).unwrap();
        assert_eq!(
            gen1.manager.inspect("mid-flight").unwrap().state,
            WorkloadState::Running
        );

        // Generation 2: a fresh daemon over the same snapshot.
        let gen2 = manager("gen2", &snapshot);
        assert_eq!(gen2.manager.reconcile(), 3, "all three records reconcile");

        // Terminal record: preserved as history, logs still readable.
        let echo_info = gen2.manager.inspect("echo").unwrap();
        assert_eq!(echo_info.state, WorkloadState::Stopped);
        assert_eq!(echo_info.exit_code, Some(0));
        let logs = gen2
            .manager
            .logs(&echo, LogsQuery::Stdout, 10)
            .unwrap()
            .join("\n");
        assert!(logs.contains("reconcile-marker"), "logs: {logs:?}");

        // Created record: still startable — the runtime never assumed.
        assert_eq!(
            gen2.manager.inspect("created-only").unwrap().state,
            WorkloadState::Created
        );
        gen2.manager.start(&created).unwrap();
        let status = gen2
            .manager
            .wait(&created, Duration::from_secs(15))
            .unwrap();
        assert_eq!(status.code, Some(0));

        // Mid-flight record: attributed, not dropped (SPEC §48).
        let flight = gen2.manager.inspect("mid-flight").unwrap();
        assert_eq!(flight.state, WorkloadState::Failed);
        assert!(flight.killed, "kill-on-close reaped it: killed must be set");
        assert_eq!(flight.exit_code, None);

        // Destroy clears reconciled records and rewrites the snapshot.
        gen2.manager.destroy(&echo).unwrap();
        gen2.manager.destroy(&created).unwrap();
        gen2.manager.destroy(&running).unwrap();
        let raw = std::fs::read_to_string(&snapshot).unwrap();
        let entries: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
        assert!(entries.is_empty(), "destroyed workloads leave the snapshot");

        std::fs::remove_dir_all(&gen1.base).ok();
        std::fs::remove_dir_all(&gen2.base).ok();
        std::fs::remove_file(&snapshot).ok();
    }

    #[test]
    fn corrupt_snapshot_is_dropped_not_trusted() {
        let snapshot =
            std::env::temp_dir().join(format!("tpt-reconcile-corrupt-{}.json", std::process::id()));
        std::fs::write(&snapshot, "{torn line").unwrap();

        let gen = manager("corrupt", &snapshot);
        assert_eq!(
            gen.manager.reconcile(),
            0,
            "corrupt snapshot yields nothing"
        );
        assert!(gen.manager.list().is_empty());
        std::fs::remove_dir_all(&gen.base).ok();
        std::fs::remove_file(&snapshot).ok();
    }

    #[test]
    fn name_conflicts_favor_live_records() {
        let snapshot =
            std::env::temp_dir().join(format!("tpt-reconcile-name-{}.json", std::process::id()));

        let gen1 = manager("name1", &snapshot);
        let id = gen1
            .manager
            .create(cmd_spec("clashing", &["echo", "old-generation"]))
            .unwrap();
        gen1.manager.start(&id).unwrap();
        gen1.manager.wait(&id, Duration::from_secs(15)).unwrap();
        drop(gen1);

        let gen2 = manager("name2", &snapshot);
        // A live workload already holds the name before reconciliation.
        let live = gen2
            .manager
            .create(cmd_spec("clashing", &["echo", "new-generation"]))
            .unwrap();
        assert_eq!(
            gen2.manager.reconcile(),
            0,
            "snapshot loses to the live record"
        );
        let info = gen2.manager.inspect("clashing").unwrap();
        assert_eq!(info.state, WorkloadState::Created);

        gen2.manager.destroy(&live).unwrap();
        // The conflicting history was discarded in favor of the live
        // record (documented semantics): after the destroy's persist,
        // nothing remains to reconcile.
        assert_eq!(gen2.manager.reconcile(), 0);
        assert!(gen2.manager.inspect("clashing").is_err());
        std::fs::remove_dir_all(&gen2.base).ok();
        std::fs::remove_file(&snapshot).ok();
    }
}
