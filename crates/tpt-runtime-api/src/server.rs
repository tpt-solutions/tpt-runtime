//! Named-pipe API server (SPEC §30).

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWrite, BufReader};
use tokio::sync::Mutex as AsyncMutex;
use tpt_runtime_config::DaemonConfig;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_ipc::{read_message, write_message, Request, Response};
use tpt_runtime_workload::manager::LogsQuery;
use tpt_runtime_workload::WorkloadManager;

/// Shared state behind the API surface.
pub struct ApiState {
    pub manager: Arc<WorkloadManager>,
    pub started_at: std::time::Instant,
    pub shutdown: tokio::sync::watch::Sender<bool>,
    /// Host GPU telemetry, when GPUs were discovered at startup.
    pub gpu: Option<Arc<tpt_runtime_gpu::GpuTelemetry>>,
}

/// Serves the local API until a client calls `daemon.shutdown` or the
/// shutdown watch fires.
pub async fn serve(config: DaemonConfig, state: Arc<ApiState>) -> Result<()> {
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&config.pipe_name)
            .map_err(|err| {
                RuntimeError::new(
                    ErrorKind::System,
                    format!("cannot create pipe '{}': {err}", config.pipe_name),
                )
            })?;
        let mut shutdown_rx = state.shutdown.subscribe();

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                connected = server.connect() => {
                    connected.map_err(|err| RuntimeError::new(
                        ErrorKind::System,
                        format!("pipe accept failed: {err}"),
                    ))?;
                    let client = server;
                    server = ServerOptions::new().create(&config.pipe_name).map_err(|err| {
                        RuntimeError::new(ErrorKind::System, format!("pipe re-create failed: {err}"))
                    })?;
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(err) = handle_connection(client, state).await {
                            eprintln!("[api] connection error: {err}");
                        }
                    });
                }
            }
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        // Non-Windows hosts use a localhost TCP socket with the same
        // framing; the primary target is Windows (SPEC §1).
        let port: u16 = config
            .pipe_name
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(7900);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|err| {
                RuntimeError::new(
                    ErrorKind::System,
                    format!("cannot bind 127.0.0.1:{port}: {err}"),
                )
            })?;
        let mut shutdown_rx = state.shutdown.subscribe();
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                accepted = listener.accept() => {
                    let (stream, _) = accepted.map_err(|err| {
                        RuntimeError::new(ErrorKind::System, format!("accept failed: {err}"))
                    })?;
                    let state = state.clone();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, state).await;
                    });
                }
            }
        }
        Ok(())
    }
}

type SharedWriter = Arc<AsyncMutex<Box<dyn AsyncWrite + Unpin + Send>>>;

async fn handle_connection<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
>(
    stream: S,
    state: Arc<ApiState>,
) -> Result<()> {
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let writer: SharedWriter = Arc::new(AsyncMutex::new(Box::new(write_half)));

    loop {
        let request: Request = match read_message(&mut reader).await {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(()), // client closed the pipe
            Err(err) => return Err(err),
        };

        // events.subscribe: answer, then push the event stream on this
        // connection until the client hangs up.
        if request.method == "events.subscribe" {
            respond(
                &writer,
                Response::ok(request.id, serde_json::json!({"subscribed": true})),
            )
            .await?;
            return push_events(state, writer).await;
        }

        let response = dispatch(&state, request).await;
        respond(&writer, response).await?;
    }
}

async fn push_events(
    state: Arc<ApiState>,
    writer: Arc<AsyncMutex<Box<dyn AsyncWrite + Unpin + Send>>>,
) -> Result<()> {
    let mut events = state.manager.events().subscribe();
    loop {
        match events.recv().await {
            Ok(event) => {
                if write_message(&mut *writer.lock().await, &event)
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

async fn respond(writer: &SharedWriter, response: Response) -> Result<()> {
    let mut guard = writer.lock().await;
    write_message(&mut **guard, &response).await
}

async fn dispatch(state: &Arc<ApiState>, request: Request) -> Response {
    let id = request.id;
    let result = route(state, &request.method, request.params).await;
    match result {
        Ok(value) => Response::ok(id, value),
        Err(err) => Response::err(id, &err),
    }
}

async fn route(
    state: &Arc<ApiState>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let manager = &state.manager;
    let params = match params {
        serde_json::Value::Null => serde_json::Value::Object(Default::default()),
        other => other,
    };
    let get = |key: &str| params.get(key).cloned().unwrap_or(serde_json::Value::Null);
    let get_str = |key: &str| -> Option<String> { get(key).as_str().map(str::to_owned) };
    let get_u64 = |key: &str| -> Option<u64> { get(key).as_u64() };

    match method {
        "daemon.status" => Ok(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_secs": state.started_at.elapsed().as_secs(),
            "workloads": manager.list().len(),
            "gpu": state.gpu.as_ref().map(|gpu| gpu.snapshot()).unwrap_or_default(),
        })),

        "workloads.create" => {
            if let Some(manifest_text) = get_str("manifest") {
                let manifest = tpt_runtime_config::Manifest::parse(&manifest_text)?;
                let spec = manifest.into_workload_spec()?;
                let id = manager.create(spec)?;
                return Ok(serde_json::json!({ "id": id.to_string() }));
            }
            Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                "workloads.create requires a 'manifest' (TOML text)",
            ))
        }

        "workloads.start" => {
            let id = resolve(manager, &required(&get_str("id"), "id")?)?;
            manager.start(&id)?;
            Ok(serde_json::json!({ "started": id.to_string() }))
        }

        "workloads.stop" => {
            let id = resolve(manager, &required(&get_str("id"), "id")?)?;
            let timeout = get_u64("timeout_secs").unwrap_or(30);
            let exit = manager.stop_and_wait(&id, Duration::from_secs(timeout))?;
            Ok(serde_json::json!({
                "stopped": id.to_string(),
                "exit_code": exit.code,
                "killed": exit.killed,
            }))
        }

        "workloads.restart" => {
            let id = resolve(manager, &required(&get_str("id"), "id")?)?;
            manager.restart(&id)?;
            Ok(serde_json::json!({ "restarted": id.to_string() }))
        }

        "workloads.pause" => {
            let id = resolve(manager, &required(&get_str("id"), "id")?)?;
            manager.pause(&id)?;
            Ok(serde_json::json!({ "paused": id.to_string() }))
        }

        "workloads.destroy" => {
            let id = resolve(manager, &required(&get_str("id"), "id")?)?;
            manager.destroy(&id)?;
            Ok(serde_json::json!({ "destroyed": id.to_string() }))
        }

        "workloads.list" => Ok(serde_json::to_value(manager.list()).unwrap_or_default()),

        "workloads.inspect" => {
            let info = manager.inspect(&required(&get_str("id"), "id")?)?;
            Ok(serde_json::to_value(info).unwrap_or_default())
        }

        "workloads.logs" => {
            let info = manager.inspect(&required(&get_str("id"), "id")?)?;
            let stream = match get_str("stream").as_deref() {
                Some("stderr") => LogsQuery::Stderr,
                _ => LogsQuery::Stdout,
            };
            let tail = get_u64("tail").unwrap_or(200) as usize;
            let lines = manager.logs(&info.id, stream, tail)?;
            Ok(serde_json::json!({ "lines": lines }))
        }

        "workloads.usage" => {
            let info = manager.inspect(&required(&get_str("id"), "id")?)?;
            let usage = manager.usage(&info.id)?;
            Ok(serde_json::to_value(usage).unwrap_or_default())
        }

        "events.history" => {
            let limit = get_u64("limit").unwrap_or(100) as usize;
            // history is served through the hub's sink; use a hub view
            Ok(serde_json::json!({ "events": manager.events().read_history(limit)? }))
        }

        "volumes.create" => {
            let name = required(&get_str("name"), "name")?;
            let volume = manager.storage().lock().unwrap().create(&name)?;
            Ok(
                serde_json::json!({ "name": volume.name, "path": volume.backing_path.display().to_string() }),
            )
        }

        "volumes.list" => {
            let volumes = manager.storage().lock().unwrap().list();
            Ok(serde_json::to_value(volumes).unwrap_or_default())
        }

        "volumes.remove" => {
            let name = required(&get_str("name"), "name")?;
            manager.storage().lock().unwrap().remove(&name)?;
            Ok(serde_json::json!({ "removed": name }))
        }

        "devices.list" => {
            let devices = manager.devices().lock().unwrap().list();
            Ok(serde_json::to_value(devices).unwrap_or_default())
        }

        "secrets.set" => {
            let name = required(&get_str("name"), "name")?;
            let value = required(&get_str("value"), "value")?;
            manager.secrets().lock().unwrap().set(&name, &value)?;
            Ok(serde_json::json!({ "set": name }))
        }

        "secrets.list" => {
            let secrets = manager.secrets().lock().unwrap().list();
            Ok(serde_json::to_value(secrets).unwrap_or_default())
        }

        "secrets.delete" => {
            let name = required(&get_str("name"), "name")?;
            let existed = manager.secrets().lock().unwrap().delete(&name)?;
            Ok(serde_json::json!({ "deleted": existed }))
        }

        "daemon.shutdown" => {
            let _ = state.shutdown.send(true);
            Ok(serde_json::json!({ "shutting_down": true }))
        }

        other => Err(RuntimeError::new(
            ErrorKind::NotFound,
            format!("unknown method '{other}'"),
        )),
    }
}

/// Resolves a workload reference (id or name) through the manager.
fn resolve(
    manager: &WorkloadManager,
    id_or_name: &str,
) -> Result<tpt_runtime_core::id::WorkloadId> {
    Ok(manager.inspect(id_or_name)?.id)
}

fn required(field: &Option<String>, key: &str) -> Result<String> {
    field.clone().ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("missing required parameter '{key}'"),
        )
    })
}
