//! Lifecycle orchestration (SPEC §26) across all backends.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tpt_runtime_capability::CapabilitySet;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::event::{EventKind, RuntimeEvent};
use tpt_runtime_core::id::WorkloadId;
use tpt_runtime_core::state::WorkloadState;
use tpt_runtime_core::timestamp::Timestamp;
use tpt_runtime_device::DeviceRegistry;
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_model::BackendKind;
use tpt_runtime_network::{NetworkAssignment, NetworkManager};
use tpt_runtime_observe::{EventHub, MetricsRegistry};
use tpt_runtime_policy::{PolicyEngine, ResourcePolicy};
use tpt_runtime_process::{ExecutionBackend, ExitStatus, ResolvedMount, StopMode, WorkloadInstance};
use tpt_runtime_security::SecretStore;
use tpt_runtime_storage::StorageManager;

use crate::info::{MountInfo, WorkloadInfo};

/// One managed workload: spec, resolved resources, live instance.
struct WorkloadRecord {
    spec: WorkloadSpec,
    state: WorkloadState,
    created_at: Timestamp,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    exit: Option<ExitStatus>,
    capabilities: CapabilitySet,
    network: Option<NetworkAssignment>,
    mounts: Vec<ResolvedMount>,
    log_dir: PathBuf,
    restarts: u32,
    instance: Option<Arc<dyn WorkloadInstance>>,
}

impl WorkloadRecord {
    fn info(&self, id: &WorkloadId, metrics: &MetricsRegistry) -> WorkloadInfo {
        let usage = metrics.get(id);
        WorkloadInfo {
            id: id.clone(),
            name: self.spec.name.clone(),
            backend: self.spec.backend(),
            state: self.state,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            exit_code: self.exit.as_ref().and_then(|e| e.code),
            killed: self.exit.as_ref().map(|e| e.killed).unwrap_or(false),
            capabilities: self.capabilities.grants().map(|g| g.name()).collect(),
            network_mode: self
                .network
                .as_ref()
                .map(|n| n.mode.to_string())
                .unwrap_or_else(|| "none".to_owned()),
            exposed_ports: self
                .network
                .as_ref()
                .map(|n| {
                    n.ports
                        .iter()
                        .map(|p| (p.name.clone(), p.host_port))
                        .collect()
                })
                .unwrap_or_default(),
            mounts: self
                .mounts
                .iter()
                .map(|m| MountInfo {
                    name: m.name.clone(),
                    mount: m.mount.clone(),
                    host_path: m.host_path.display().to_string(),
                    mode: m.mode.to_string(),
                })
                .collect(),
            usage,
            details: self.instance.as_ref().map(|i| i.describe()),
            restarts: self.restarts,
        }
    }
}

/// Which log stream to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogsQuery {
    /// Combined stdout.
    Stdout,
    /// Combined stderr.
    Stderr,
}

/// The workload manager: owns backends and orchestrates the lifecycle
/// (SPEC §26: create/start/pause/resume/restart/stop/kill/destroy).
pub struct WorkloadManager {
    backends: Mutex<BTreeMap<BackendKind, Arc<dyn ExecutionBackend>>>,
    records: Arc<Mutex<BTreeMap<WorkloadId, WorkloadRecord>>>,
    logs_base: PathBuf,
    events: Arc<EventHub>,
    metrics: Arc<MetricsRegistry>,
    storage: Arc<Mutex<StorageManager>>,
    network: Arc<NetworkManager>,
    devices: Arc<Mutex<DeviceRegistry>>,
    secrets: Arc<Mutex<SecretStore>>,
    policy: PolicyEngine,
}

impl WorkloadManager {
    /// Assembles a manager over shared subsystems.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        logs_base: impl Into<PathBuf>,
        events: Arc<EventHub>,
        metrics: Arc<MetricsRegistry>,
        storage: Arc<Mutex<StorageManager>>,
        network: Arc<NetworkManager>,
        devices: Arc<Mutex<DeviceRegistry>>,
        secrets: Arc<Mutex<SecretStore>>,
        policy: PolicyEngine,
    ) -> Self {
        Self {
            backends: Mutex::new(BTreeMap::new()),
            records: Arc::new(Mutex::new(BTreeMap::new())),
            logs_base: logs_base.into(),
            events,
            metrics,
            storage,
            network,
            devices,
            secrets,
            policy,
        }
    }

    /// Registers an execution backend under its kind.
    pub fn register_backend(&self, backend: Arc<dyn ExecutionBackend>) {
        self.backends.lock().unwrap().insert(backend.kind(), backend);
    }

    fn backend(&self, kind: BackendKind) -> Result<Arc<dyn ExecutionBackend>> {
        self.backends
            .lock()
            .unwrap()
            .get(&kind)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::BackendUnavailable,
                    format!("backend '{kind}' is not registered on this host"),
                )
            })
    }

    /// Creates a workload: validates, admits via policy, resolves volumes,
    /// network, devices and capabilities, prepares the backend. The
    /// workload sits in `Created` afterwards (SPEC §3.1).
    pub fn create(&self, spec: WorkloadSpec) -> Result<WorkloadId> {
        spec.validate()?;
        self.policy.enforce(&spec)?;

        let id = WorkloadId::generate();
        {
            let records = self.records.lock().unwrap();
            if records
                .values()
                .any(|r| r.spec.name == spec.name && r.state != WorkloadState::Destroyed)
            {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    format!("workload name '{}' is already in use", spec.name),
                ));
            }
        }

        // Resolve logical volumes to host paths (SPEC §15).
        let mut mounts = Vec::new();
        for mount in &spec.volumes {
            let storage = self.storage.lock().unwrap();
            let host_path = storage.mount_path(&mount.name)?;
            mounts.push(ResolvedMount {
                name: mount.name.clone(),
                mount: mount.mount.clone(),
                host_path,
                mode: mount.mode,
            });
        }

        // Resolve network intent (SPEC §18).
        let network = if spec.network.mode == tpt_runtime_model::network::NetworkMode::None {
            None
        } else {
            Some(block_on(self.network.assign(&spec.name, &spec.network))?)
        };

        // Capabilities: explicit grants only (SPEC §5.2).
        let capabilities = CapabilitySet::from_names(spec.capabilities.iter().map(String::as_str));

        // Devices must exist to be attached (SPEC §19).
        for device in &spec.devices {
            self.devices
                .lock()
                .unwrap()
                .attach(&spec.name, &device.id, device.mode)?;
        }

        // Secrets referenced by capability must exist (SPEC §24).
        for name in &spec.capabilities {
            if let Some(secret_name) = name.strip_prefix("secret:") {
                let secrets = self.secrets.lock().unwrap();
                secrets.resolve(&tpt_runtime_capability::Capability::Secret {
                    name: secret_name.to_owned(),
                })?;
            }
        }

        let ctx = self.build_context(&id, &mounts, &network, &capabilities);
        self.backend(spec.backend())?.prepare(&spec, &ctx)?;

        self.records.lock().unwrap().insert(
            id.clone(),
            WorkloadRecord {
                spec,
                state: WorkloadState::Created,
                created_at: Timestamp::now(),
                started_at: None,
                finished_at: None,
                exit: None,
                capabilities,
                network,
                mounts,
                log_dir: ctx.log_dir.clone(),
                restarts: 0,
                instance: None,
            },
        );

        self.emit(EventKind::WorkloadCreated, &id);
        self.emit(EventKind::WorkloadResolved, &id);
        self.emit(EventKind::WorkloadPrepared, &id);
        Ok(id)
    }

    /// Starts a `Created` (or `Stopped`) workload.
    pub fn start(&self, id: &WorkloadId) -> Result<()> {
        let (spec, ctx) = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(id).ok_or_else(|| not_found(id))?;
            match record.state {
                WorkloadState::Created | WorkloadState::Stopped => {}
                other => {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidTransition,
                        format!("cannot start workload in state '{other}'"),
                    )
                    .with_workload(id.clone()))
                }
            }
            record.state = WorkloadState::Starting;
            record.exit = None;
            record.finished_at = None;
            if record.started_at.is_some() {
                record.restarts += 1;
            }
            let ctx = self.build_context(id, &record.mounts, &record.network, &record.capabilities);
            (record.spec.clone(), ctx)
        };
        let capabilities = ctx.capabilities.clone();
        let network = {
            let records = self.records.lock().unwrap();
            records.get(id).and_then(|r| r.network.clone())
        };

        let backend = self.backend(spec.backend())?;

        let instance = match backend.start(&spec, &ctx) {
            Ok(instance) => instance,
            Err(err) => {
                let mut records = self.records.lock().unwrap();
                if let Some(record) = records.get_mut(id) {
                    record.state = WorkloadState::Failed;
                    record.finished_at = Some(Timestamp::now());
                }
                self.emit(EventKind::WorkloadFailed, id);
                return Err(err);
            }
        };
        let instance: Arc<dyn WorkloadInstance> = Arc::from(instance);

        // Granted events fire on the start boundary so audits line up with
        // execution (SPEC §23 attribution).
        for grant in capabilities.grants() {
            self.events.emit(
                RuntimeEvent::now(EventKind::CapabilityGranted)
                    .with_workload(id.clone())
                    .with_field("capability", grant.name()),
            );
        }
        if let Some(network) = &network {
            if !network.ports.is_empty() {
                self.events.emit(
                    RuntimeEvent::now(EventKind::NetworkConnected)
                        .with_workload(id.clone())
                        .with_field(
                            "ports",
                            serde_json::json!(network
                                .ports
                                .iter()
                                .map(|p| (p.name.clone(), p.host_port))
                                .collect::<Vec<_>>()),
                        ),
                );
            }
        }

        {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(id).expect("record exists");
            record.state = WorkloadState::Running;
            record.started_at = Some(Timestamp::now());
            record.instance = Some(instance.clone());
            record.log_dir = ctx.log_dir.clone();
        }

        self.emit(EventKind::WorkloadStarted, id);
        self.spawn_watcher(id.clone(), instance, network);
        Ok(())
    }

    /// Waits up to `timeout` for a running workload to exit on its own.
    pub fn wait(&self, id: &WorkloadId, timeout: Duration) -> Result<ExitStatus> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let records = self.records.lock().unwrap();
                if let Some(record) = records.get(id) {
                    if !record.state.is_active() {
                        return Ok(record.exit.clone().unwrap_or_else(ExitStatus::success));
                    }
                } else {
                    return Err(not_found(id));
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(RuntimeError::new(
                    ErrorKind::System,
                    "workload did not exit in time",
                )
                .with_workload(id.clone())
                .with_operation("wait"));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Stops a workload and waits up to `timeout` for it to settle.
    pub fn stop_and_wait(&self, id: &WorkloadId, timeout: Duration) -> Result<ExitStatus> {
        let instance = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(id).ok_or_else(|| not_found(id))?;
            match record.state {
                WorkloadState::Stopped | WorkloadState::Failed => {
                    return Ok(record.exit.clone().unwrap_or_else(ExitStatus::success))
                }
                WorkloadState::Created => {
                    // nothing was ever started; treat as settled
                    return Ok(ExitStatus::success());
                }
                WorkloadState::Starting | WorkloadState::Running | WorkloadState::Paused => {
                    record.state = WorkloadState::Stopping;
                    record.instance.clone()
                }
                other => {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidTransition,
                        format!("cannot stop workload in state '{other}'"),
                    )
                    .with_workload(id.clone()))
                }
            }
        };
        let instance = instance.ok_or_else(|| {
            RuntimeError::new(ErrorKind::System, "running workload has no live instance")
                .with_workload(id.clone())
        })?;
        instance.stop(StopMode::Kill)?;

        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let records = self.records.lock().unwrap();
                if let Some(record) = records.get(id) {
                    if !record.state.is_active() {
                        return Ok(record.exit.clone().unwrap_or_else(ExitStatus::success));
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(RuntimeError::new(
                    ErrorKind::System,
                    "workload did not stop in time",
                )
                .with_workload(id.clone())
                .with_operation("stop"));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Restarts a stopped or running workload (SPEC §26 restart).
    pub fn restart(&self, id: &WorkloadId) -> Result<()> {
        let active = {
            let records = self.records.lock().unwrap();
            records
                .get(id)
                .map(|r| r.state.is_active())
                .ok_or_else(|| not_found(id))?
        };
        if active {
            self.stop_and_wait(id, Duration::from_secs(10))?;
        }
        self.start(id)
    }

    /// Pauses a workload. No MVP backend implements suspension; the error
    /// is explicit rather than a silent no-op (SPEC §48).
    pub fn pause(&self, id: &WorkloadId) -> Result<()> {
        let records = self.records.lock().unwrap();
        let record = records.get(id).ok_or_else(|| not_found(id))?;
        Err(RuntimeError::new(
            ErrorKind::NotImplemented,
            "no registered backend supports pausing yet",
        )
        .with_workload(id.clone())
        .with_backend(record.spec.backend().to_string()))
    }

    /// Removes a finished workload's records and resources.
    pub fn destroy(&self, id: &WorkloadId) -> Result<()> {
        let _ = self.stop_and_wait(id, Duration::from_secs(5));

        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(id).ok_or_else(|| not_found(id))?;
        // Created workloads never ran: destroying them is legal cleanup.
        if record.state.is_active() && record.state != WorkloadState::Created {
            return Err(RuntimeError::new(
                ErrorKind::InvalidTransition,
                "workload is still active",
            )
            .with_workload(id.clone()));
        }
        record.state = WorkloadState::Destroyed;
        self.emit(EventKind::WorkloadDestroyed, id);
        self.metrics.remove(id);
        records.remove(id);
        Ok(())
    }

    /// Lists all workloads.
    pub fn list(&self) -> Vec<WorkloadInfo> {
        let records = self.records.lock().unwrap();
        records
            .iter()
            .map(|(id, r)| r.info(id, &self.metrics))
            .collect()
    }

    /// Inspects one workload by id or name.
    pub fn inspect(&self, id_or_name: &str) -> Result<WorkloadInfo> {
        let records = self.records.lock().unwrap();
        records
            .iter()
            .find(|(id, r)| id_matches(id, id_or_name) || r.spec.name == id_or_name)
            .map(|(id, r)| r.info(id, &self.metrics))
            .ok_or_else(|| not_found_str(id_or_name))
    }

    /// Latest resource usage for a workload, sampling the live instance.
    pub fn usage(&self, id: &WorkloadId) -> Result<tpt_runtime_core::ResourceUsage> {
        let instance = {
            let records = self.records.lock().unwrap();
            let record = records.get(id).ok_or_else(|| not_found(id))?;
            record.instance.clone()
        };
        let usage = match instance {
            Some(instance) => instance.stats()?,
            None => tpt_runtime_core::ResourceUsage::default(),
        };
        self.metrics.update(id, usage.clone());
        Ok(usage)
    }

    /// Reads the tail of a workload's log stream.
    pub fn logs(&self, id: &WorkloadId, stream: LogsQuery, tail: usize) -> Result<Vec<String>> {
        let (log_dir, name) = {
            let records = self.records.lock().unwrap();
            let record = records.get(id).ok_or_else(|| not_found(id))?;
            (record.log_dir.clone(), record.spec.name.clone())
        };
        let (file, label) = match stream {
            LogsQuery::Stdout => ("stdout.log", "stdout"),
            LogsQuery::Stderr => ("stderr.log", "stderr"),
        };
        let raw = std::fs::read_to_string(log_dir.join(file)).map_err(|err| {
            RuntimeError::new(
                ErrorKind::NotFound,
                format!("no {label} log for '{name}': {err}"),
            )
        })?;
        let start = raw.lines().count().saturating_sub(tail);
        Ok(raw.lines().skip(start).map(str::to_owned).collect())
    }

    /// Access to the event hub (API event streaming).
    pub fn events(&self) -> &EventHub {
        &self.events
    }

    /// Access to the metrics registry.
    pub fn metrics(&self) -> &MetricsRegistry {
        &self.metrics
    }

    /// Device registry (`tpt device list`, GPU registration).
    pub fn devices(&self) -> &Arc<Mutex<DeviceRegistry>> {
        &self.devices
    }

    /// Storage manager (`tpt volume ...`).
    pub fn storage(&self) -> &Arc<Mutex<StorageManager>> {
        &self.storage
    }

    /// Network manager.
    pub fn network(&self) -> &Arc<NetworkManager> {
        &self.network
    }

    /// Secret store (`tpt secret ...`).
    pub fn secrets(&self) -> &Arc<Mutex<SecretStore>> {
        &self.secrets
    }

    /// Builder-style override of the default admission policy.
    pub fn set_default_policy(&mut self, policy: ResourcePolicy) {
        self.policy = PolicyEngine::new(tpt_runtime_policy::HostCapacity::unknown())
            .with_default_policy(policy);
    }

    fn build_context(
        &self,
        id: &WorkloadId,
        mounts: &[ResolvedMount],
        network: &Option<NetworkAssignment>,
        capabilities: &CapabilitySet,
    ) -> tpt_runtime_process::StartContext {
        tpt_runtime_process::StartContext {
            workload_id: id.clone(),
            mounts: mounts.to_vec(),
            log_dir: self.logs_base.join(id.as_str()),
            capabilities: capabilities.clone(),
            network_mode: network
                .as_ref()
                .map(|n| n.mode)
                .unwrap_or(tpt_runtime_model::network::NetworkMode::None),
            exposed_ports: network
                .as_ref()
                .map(|n| {
                    n.ports
                        .iter()
                        .map(|p| (p.name.clone(), p.host_port))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn spawn_watcher(
        &self,
        id: WorkloadId,
        instance: Arc<dyn WorkloadInstance>,
        network: Option<NetworkAssignment>,
    ) {
        let events = self.events.clone();
        let metrics = self.metrics.clone();
        let network_manager = self.network.clone();
        let records = self.records.clone();

        std::thread::spawn(move || {
            let exit_rx = instance.exit();
            let status = match tokio::runtime::Builder::new_current_thread().build() {
                Ok(rt) => rt
                    .block_on(async { exit_rx.await.unwrap_or(ExitStatus::terminated()) }),
                Err(_) => ExitStatus::terminated(),
            };

            if let Ok(usage) = instance.stats() {
                metrics.update(&id, usage);
            }
            if let Some(network) = network {
                network_manager.release(&network);
            }
            if let Ok(mut guard) = records.lock() {
                if let Some(record) = guard.get_mut(&id) {
                    record.exit = Some(status.clone());
                    record.finished_at = Some(Timestamp::now());
                    record.state = if status.failed {
                        WorkloadState::Failed
                    } else {
                        WorkloadState::Stopped
                    };
                }
            }

            events.emit(
                RuntimeEvent::now(if status.failed {
                    EventKind::WorkloadFailed
                } else {
                    EventKind::WorkloadStopped
                })
                .with_workload(id.clone())
                .with_field("exit_code", status.code)
                .with_field("killed", status.killed),
            );
        });
    }

    fn emit(&self, kind: EventKind, id: &WorkloadId) {
        self.events
            .emit(RuntimeEvent::now(kind).with_workload(id.clone()));
    }
}

/// Lookup helper on ids without adding methods to the core type.
fn id_matches(id: &WorkloadId, id_or_name: &str) -> bool {
    id.as_str() == id_or_name
}

fn not_found(id: &WorkloadId) -> RuntimeError {
    RuntimeError::new(ErrorKind::NotFound, format!("workload '{id}' not found"))
}

fn not_found_str(id: &str) -> RuntimeError {
    RuntimeError::new(ErrorKind::NotFound, format!("workload '{id}' not found"))
}

/// Blocking on bounded async work (network assignment) from sync paths.
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("temporary runtime")
        .block_on(future)
}
