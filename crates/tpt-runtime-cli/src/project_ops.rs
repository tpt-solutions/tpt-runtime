//! Project operations behind `tpt up` / `tpt down` (SPEC §33, §34).
//!
//! Pure API orchestration: parse the project file, then create/start (or
//! stop/destroy) workloads through the daemon in dependency order. No
//! runtime logic lives here — the CLI stays an API client (SPEC §30).

use anyhow::{bail, Context, Result};
use tpt_runtime_api::ApiClient;
use tpt_runtime_config::{materialize, Project};

/// One workload brought up by [`up`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct Started {
    /// Project-local workload name.
    pub name: String,
    /// Runtime workload id.
    pub id: String,
    /// Exposed ports as `(logical name, host port)`.
    pub ports: Vec<(String, u16)>,
}

/// Starts the project's workloads in dependency order. With `only`, starts
/// just that workload plus its transitive dependencies. Already-started
/// workloads keep running when a later one fails (compose semantics).
pub async fn up(
    client: &mut ApiClient,
    project: &Project,
    only: Option<&str>,
) -> Result<Vec<Started>> {
    let order = project
        .start_order(only)
        .map_err(|err| anyhow::anyhow!("{err}"))?;

    let mut started = Vec::new();
    for entry in order {
        let manifest = materialize(&entry.manifest, &entry.name, &project.name)
            .map_err(|err| anyhow::anyhow!("workload '{}': {err}", entry.name))?;
        let result: anyhow::Result<Started> = async {
            let created = client
                .call(
                    "workloads.create",
                    serde_json::json!({ "manifest": manifest }),
                )
                .await?;
            let id = created["id"]
                .as_str()
                .context("daemon returned no workload id")?
                .to_owned();
            client
                .call("workloads.start", serde_json::json!({ "id": id }))
                .await?;
            let info = client
                .call("workloads.inspect", serde_json::json!({ "id": id }))
                .await?;
            let ports = info["exposed_ports"]
                .as_object()
                .map(|ports| {
                    ports
                        .iter()
                        .map(|(name, port)| (name.clone(), port.as_u64().unwrap_or(0) as u16))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Started {
                name: entry.name.clone(),
                id,
                ports,
            })
        }
        .await;

        match result {
            Ok(item) => {
                let ports = item
                    .ports
                    .iter()
                    .map(|(name, port)| format!("{name}:{port}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                if ports.is_empty() {
                    println!("started {} ({})", item.name, item.id);
                } else {
                    println!("started {} ({}) on {}", item.name, item.id, ports);
                }
                started.push(item);
            }
            Err(err) => {
                if started.is_empty() {
                    bail!("workload '{}' failed to start: {err:#}", entry.name);
                }
                bail!(
                    "workload '{}' failed to start: {err:#}\n{} workload(s) are still running: {}",
                    entry.name,
                    started.len(),
                    started
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }
    Ok(started)
}

/// Stops and destroys every running workload belonging to the project, in
/// reverse dependency order. Returns the stopped names.
pub async fn down(client: &mut ApiClient, project: &Project) -> Result<Vec<String>> {
    let list = client
        .call("workloads.list", serde_json::Value::Null)
        .await?;
    let mine: Vec<(String, String)> = list
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            item["labels"][tpt_runtime_config::PROJECT_LABEL].as_str()
                == Some(project.name.as_str())
        })
        .filter_map(|item| {
            let name = item["name"].as_str()?.to_owned();
            let id = item["id"].as_str()?.to_owned();
            Some((name, id))
        })
        .collect();

    if mine.is_empty() {
        println!("no running workloads for project '{}'", project.name);
        return Ok(Vec::new());
    }

    // Stop in reverse dependency order (fall back to list order for
    // workloads no longer declared in the project file).
    let order: Vec<String> = project
        .stop_order()
        .map_err(|err| anyhow::anyhow!("{err}"))?
        .into_iter()
        .map(|w| w.name.clone())
        .collect();
    let mut ordered = order
        .into_iter()
        .filter_map(|name| {
            mine.iter()
                .find(|(project_name, _)| project_name == &name)
                .cloned()
        })
        .collect::<Vec<_>>();
    for entry in &mine {
        if !ordered.iter().any(|(name, _)| name == &entry.0) {
            ordered.push(entry.clone());
        }
    }

    let mut stopped = Vec::new();
    for (name, id) in ordered {
        // A workload that already exited on its own still needs destroy.
        let _ = client
            .call("workloads.stop", serde_json::json!({ "id": id }))
            .await;
        client
            .call("workloads.destroy", serde_json::json!({ "id": id }))
            .await
            .with_context(|| format!("workload '{name}' could not be destroyed"))?;
        println!("stopped {name} ({id})");
        stopped.push(name);
    }
    Ok(stopped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn templates_are_valid_projects_with_valid_manifests() {
        for (text, name, count) in [
            (include_str!("templates/tpt-minimal.toml"), "hello-tpt", 1),
            (
                include_str!("templates/tpt-services.toml"),
                "two-services",
                2,
            ),
            (
                include_str!("templates/tpt-wasm.toml"),
                "wasm-quickstart",
                1,
            ),
        ] {
            let project = Project::parse(text, Path::new(".")).unwrap();
            assert_eq!(project.name, name);
            assert_eq!(project.workloads().len(), count);
            // Dependency ordering terminates and every manifest (inline or
            // external-shaped) parses and survives materialization.
            assert!(project.start_order(None).unwrap().len() == count);
            for workload in project.workloads() {
                tpt_runtime_config::Manifest::parse(&workload.manifest)
                    .unwrap_or_else(|err| panic!("{}: {err}", workload.name));
                let materialized =
                    materialize(&workload.manifest, &workload.name, &project.name).unwrap();
                let manifest = tpt_runtime_config::Manifest::parse(&materialized).unwrap();
                assert_eq!(
                    manifest
                        .labels
                        .get(tpt_runtime_config::PROJECT_LABEL)
                        .map(String::as_str),
                    Some(project.name.as_str())
                );
                assert_eq!(manifest.workload.name, workload.name);
            }
        }
    }

    #[test]
    fn services_template_declares_the_dependency() {
        let project =
            Project::parse(include_str!("templates/tpt-services.toml"), Path::new(".")).unwrap();
        let api = project.get("api").unwrap();
        assert_eq!(api.depends_on, ["database"]);
        let order = project.start_order(None).unwrap();
        assert_eq!(order[0].name, "database");
        assert_eq!(order[1].name, "api");
    }
}

/// End-to-end: project up/down drives a real manager over a real pipe
/// (SPEC §45 integration, developer-platform row).
#[cfg(all(test, windows))]
mod e2e {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tpt_runtime_api::server::{serve, ApiState};
    use tpt_runtime_config::DaemonConfig;
    use tpt_runtime_core::event::EventKind;
    use tpt_runtime_device::DeviceRegistry;
    use tpt_runtime_network::NetworkManager;
    use tpt_runtime_observe::{EventHub, MetricsRegistry};
    use tpt_runtime_policy::PolicyEngine;
    use tpt_runtime_security::SecretStore;
    use tpt_runtime_storage::StorageManager;
    use tpt_runtime_windows::WindowsProcessBackend;
    use tpt_runtime_workload::WorkloadManager;

    async fn stack(tag: &str) -> (DaemonConfig, Arc<WorkloadManager>, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("tpt-updown-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut config = DaemonConfig::from_env();
        config.pipe_name = format!(r"\\.\pipe\tpt-test-updown-{}-{}", tag, std::process::id());
        config.state_dir = base.clone();

        let manager = Arc::new(WorkloadManager::new(
            base.join("logs"),
            Arc::new(EventHub::new(None::<std::path::PathBuf>)),
            Arc::new(MetricsRegistry::new()),
            Arc::new(std::sync::Mutex::new(
                StorageManager::open(base.join("volumes")).unwrap(),
            )),
            Arc::new(NetworkManager::new()),
            Arc::new(std::sync::Mutex::new(DeviceRegistry::new())),
            Arc::new(std::sync::Mutex::new(
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
        let serve_config = config.clone();
        tokio::spawn(async move {
            let _ = serve(serve_config, state).await;
        });
        (config, manager, base)
    }

    fn echo_workload(name: &str, deps: &[&str]) -> String {
        format!(
            r#"
[[workload]]
name = "{name}"
{}
[workload.manifest.execution]
backend = "windows"
program = "cmd.exe"
args = ["/C", "echo up-down-{name}"]
"#,
            if deps.is_empty() {
                String::new()
            } else {
                format!(
                    "depends_on = [{}]\n",
                    deps.iter()
                        .map(|d| format!("\"{d}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn up_and_down_drive_a_real_daemon_stack() {
        let (config, manager, base) = stack("full").await;
        let mut client = ApiClient::connect(&config).await.unwrap();

        let project_text = format!(
            "[project]\nname = \"e2e-proj\"\n{}{}",
            echo_workload("database", &[]),
            echo_workload("api", &["database"])
        );
        let project = Project::parse(&project_text, std::path::Path::new(".")).unwrap();

        // up: dependency first, both labeled with the project
        let started = up(&mut client, &project, None).await.unwrap();
        assert_eq!(
            started.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["database", "api"]
        );

        let info = manager.inspect("api").unwrap();
        assert_eq!(
            info.labels
                .get(tpt_runtime_config::PROJECT_LABEL)
                .map(String::as_str),
            Some("e2e-proj")
        );

        // up is idempotent-ish: a second up fails on duplicate names with
        // attribution, leaving the first pair running.
        let second = up(&mut client, &project, None).await;
        assert!(second.is_err(), "duplicate up must fail on name conflict");

        // down: everything belonging to the project is stopped + destroyed
        let stopped = down(&mut client, &project).await.unwrap();
        assert_eq!(stopped.len(), 2, "stopped: {stopped:?}");
        assert!(manager.list().is_empty());

        // A second down finds nothing and succeeds.
        let stopped = down(&mut client, &project).await.unwrap();
        assert!(stopped.is_empty());

        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn up_only_starts_a_workload_and_its_dependencies() {
        let (config, manager, base) = stack("only").await;
        let mut client = ApiClient::connect(&config).await.unwrap();

        let project_text = format!(
            "[project]\nname = \"only-proj\"\n{}{}{}{}",
            echo_workload("leaf", &["mid"]),
            echo_workload("mid", &["root"]),
            echo_workload("root", &[]),
            echo_workload("standalone", &[]),
        );
        let project = Project::parse(&project_text, std::path::Path::new(".")).unwrap();

        let started = up(&mut client, &project, Some("leaf")).await.unwrap();
        assert_eq!(
            started.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["root", "mid", "leaf"]
        );
        assert!(manager.inspect("standalone").is_err());

        down(&mut client, &project).await.unwrap();
        std::fs::remove_dir_all(&base).ok();
    }

    // Keep the EventKind import honest: up/down should emit lifecycle
    // events through the manager even though this test asserts state only.
    #[allow(dead_code)]
    fn event_kinds_exist(kind: EventKind) -> EventKind {
        kind
    }

    // Silence unused-import warnings for BTreeMap (labels type reference).
    #[allow(dead_code)]
    fn labels_type(_: BTreeMap<String, String>) {}
}
