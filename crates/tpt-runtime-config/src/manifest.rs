//! The `tpt.runtime/v1` workload manifest (SPEC §32, §49).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_model::device::{DeviceAccessMode, DeviceRequest};
use tpt_runtime_model::execution::{WasmClass, WindowsProcessSpec};
use tpt_runtime_model::network::NetworkSpec;
use tpt_runtime_model::resources::ResourceSpec;
use tpt_runtime_model::volume::{VolumeAccessMode, VolumeMount};
use tpt_runtime_model::workload::WorkloadSpec;
use tpt_runtime_model::{BackendKind, ExecutionSpec};

/// Manifest API version implemented by this runtime (SPEC §49).
pub const MANIFEST_API_VERSION: &str = "tpt.runtime/v1";

/// The declarative workload manifest.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest API version; must equal [`MANIFEST_API_VERSION`].
    #[serde(default = "default_api")]
    pub api: String,
    /// Workload identity.
    pub workload: WorkloadTable,
    /// Execution definition.
    pub execution: ExecutionTable,
    /// Resource requests.
    #[serde(default)]
    pub resources: ResourceSpec,
    /// Logical network request.
    #[serde(default)]
    pub network: NetworkSpec,
    /// Volume mounts.
    #[serde(default)]
    pub volumes: Vec<VolumeTable>,
    /// Device requests.
    #[serde(default)]
    pub devices: Vec<DeviceTable>,
    /// Requested capabilities, by name.
    #[serde(default)]
    pub capabilities: Vec<CapabilityTable>,
    /// Environment variables for the workload.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Free-form labels.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

fn default_api() -> String {
    MANIFEST_API_VERSION.to_owned()
}

/// `[workload]` table.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadTable {
    /// Workload name.
    pub name: String,
}

/// `[execution]` table: a `backend` selector plus that backend's fields.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTable {
    /// Backend kind (`windows`, `linux`, `oci`, `wasm`).
    pub backend: Option<String>,
    // windows
    /// Executable path (windows backend).
    #[serde(default)]
    pub program: Option<String>,
    // wasm
    /// Module path (wasm backend).
    #[serde(default)]
    pub module: Option<String>,
    /// Runtime class (wasm backend): `command`, `service`, `function`.
    #[serde(default)]
    pub class: Option<WasmClass>,
    // oci
    /// Image reference (oci backend).
    #[serde(default)]
    pub image: Option<String>,
    // linux
    /// Distro name (linux backend).
    #[serde(default)]
    pub distro: Option<String>,
    /// Command to execute (linux backend).
    #[serde(default)]
    pub command: Option<String>,
    // shared
    /// Argument vector.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory (windows backend).
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
}

/// `[[volumes]]` entry.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeTable {
    /// Logical volume name.
    pub name: String,
    /// Mount point inside the workload.
    pub mount: String,
    /// `read-only` or `read-write`.
    #[serde(default)]
    pub mode: Option<String>,
}

/// `[[devices]]` entry.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceTable {
    /// Logical device id (`gpu:0`).
    pub id: String,
    /// `compute`, `full` or `read-only`.
    #[serde(default)]
    pub mode: Option<String>,
}

/// `[[capabilities]]` entry.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityTable {
    /// Capability name, e.g. `network.outbound`.
    pub name: String,
}

impl Manifest {
    /// Parses a manifest from TOML text.
    pub fn parse(toml_text: &str) -> Result<Manifest> {
        let manifest: Manifest = toml::from_str(toml_text)
            .map_err(|err| RuntimeError::new(ErrorKind::InvalidConfiguration, format!("invalid manifest: {err}")))?;
        manifest.validate_version()?;
        Ok(manifest)
    }

    /// Parses a manifest from a TOML file.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Manifest> {
        let text = std::fs::read_to_string(path.as_ref()).map_err(|err| {
            RuntimeError::new(
                ErrorKind::NotFound,
                format!("cannot read manifest {}: {err}", path.as_ref().display()),
            )
        })?;
        Self::parse(&text)
    }

    /// Checks the declared API version (SPEC §49).
    pub fn validate_version(&self) -> Result<()> {
        if self.api != MANIFEST_API_VERSION {
            return Err(RuntimeError::new(
                ErrorKind::InvalidConfiguration,
                format!(
                    "unsupported manifest api '{}': this runtime implements '{MANIFEST_API_VERSION}'",
                    self.api
                ),
            ));
        }
        Ok(())
    }

    /// Converts the manifest into a backend-independent [`WorkloadSpec`].
    pub fn into_workload_spec(self) -> Result<WorkloadSpec> {
        self.validate_version()?;

        let execution = self.build_execution_spec()?;
        let mut spec = WorkloadSpec {
            name: self.workload.name,
            execution,
            resources: self.resources,
            network: self.network,
            volumes: Vec::with_capacity(self.volumes.len()),
            devices: Vec::with_capacity(self.devices.len()),
            capabilities: self.capabilities.into_iter().map(|c| c.name).collect(),
            labels: self.labels,
        };

        for volume in self.volumes {
            let mode = match volume.mode.as_deref() {
                None => VolumeAccessMode::ReadWrite,
                Some(text) => VolumeAccessMode::from_str(text).map_err(|err| {
                    RuntimeError::new(ErrorKind::InvalidConfiguration, err)
                })?,
            };
            spec.volumes.push(VolumeMount {
                name: volume.name,
                mount: volume.mount,
                mode,
            });
        }

        for device in self.devices {
            let mode = match device.mode.as_deref() {
                None => DeviceAccessMode::Compute,
                Some(text) => DeviceAccessMode::from_str(text).map_err(|err| {
                    RuntimeError::new(ErrorKind::InvalidConfiguration, err)
                })?,
            };
            spec.devices.push(DeviceRequest {
                id: device.id,
                mode,
            });
        }

        // Manifest-level env always applies to the execution payload.
        match &mut spec.execution {
            ExecutionSpec::WindowsProcess(s) => s.env.extend(self.env),
            ExecutionSpec::LinuxProcess(s) => s.env.extend(self.env),
            ExecutionSpec::OciImage(s) => s.env.extend(self.env),
            ExecutionSpec::WasmModule(s) => s.env.extend(self.env),
        }

        spec.validate()?;
        Ok(spec)
    }

    fn build_execution_spec(&self) -> Result<ExecutionSpec> {
        let backend = match self.execution.backend.as_deref() {
            Some(text) => BackendKind::from_str(text)
                .map_err(|err| RuntimeError::new(ErrorKind::InvalidConfiguration, err))?,
            None => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidConfiguration,
                    "manifest [execution] requires a backend",
                ))
            }
        };

        let exec = match backend {
            BackendKind::Windows => ExecutionSpec::WindowsProcess(WindowsProcessSpec {
                program: required(&self.execution.program, "execution.program")?,
                args: self.execution.args.clone(),
                working_dir: self.execution.working_dir.clone(),
                env: BTreeMap::new(),
            }),
            BackendKind::Linux => ExecutionSpec::LinuxProcess(tpt_runtime_model::execution::LinuxProcessSpec {
                distro: required(&self.execution.distro, "execution.distro")?,
                command: self
                    .execution
                    .command
                    .as_deref()
                    .map(split_command)
                    .unwrap_or_default(),
                env: BTreeMap::new(),
            }),
            BackendKind::Oci => ExecutionSpec::OciImage(tpt_runtime_model::execution::OciImageSpec {
                image: required(&self.execution.image, "execution.image")?,
                args: self.execution.args.clone(),
                env: BTreeMap::new(),
            }),
            BackendKind::Wasm => ExecutionSpec::WasmModule(tpt_runtime_model::execution::WasmModuleSpec {
                module: PathBuf::from(required(&self.execution.module, "execution.module")?),
                class: self.execution.class.unwrap_or_default(),
                env: BTreeMap::new(),
                args: self.execution.args.clone(),
            }),
        };
        Ok(exec)
    }
}

fn required(field: &Option<String>, key: &str) -> Result<String> {
    field.clone().ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::InvalidConfiguration,
            format!("manifest key '{key}' is required for the selected backend"),
        )
    })
}

fn split_command(command: &str) -> Vec<String> {
    command.split_whitespace().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_runtime_model::network::NetworkMode;

    const FULL_MANIFEST: &str = r#"
api = "tpt.runtime/v1"

[workload]
name = "example"

[execution]
backend = "windows"
program = "myapp.exe"
args = ["--port", "8080"]
working_dir = "C:/tmp"

[resources]
cpu = 4
memory = "4GiB"
timeout_secs = 600

[network]
mode = "service"

[network.expose]
http = 8080

[[volumes]]
name = "source"
mount = "/workspace"
mode = "read-only"

[[devices]]
id = "gpu:0"
mode = "compute"

[[capabilities]]
name = "network.outbound"

[labels]
team = "platform"

[env]
LOG_LEVEL = "debug"
"#;

    #[test]
    fn parses_full_manifest() {
        let manifest = Manifest::parse(FULL_MANIFEST).unwrap();
        assert_eq!(manifest.api, "tpt.runtime/v1");
        let spec = manifest.into_workload_spec().unwrap();
        assert_eq!(spec.name, "example");
        assert_eq!(spec.backend(), BackendKind::Windows);
        assert_eq!(spec.resources.memory, Some(tpt_runtime_model::Memory::gib(4)));
        assert_eq!(spec.network.mode, NetworkMode::Service);
        assert_eq!(spec.volumes[0].mode, VolumeAccessMode::ReadOnly);
        assert_eq!(spec.devices[0].id, "gpu:0");
        assert!(spec.capabilities.contains(&"network.outbound".to_owned()));
        assert_eq!(spec.labels["team"], "platform");

        match &spec.execution {
            ExecutionSpec::WindowsProcess(proc) => {
                assert_eq!(proc.program, "myapp.exe");
                assert_eq!(proc.args, ["--port", "8080"]);
                assert_eq!(proc.env.get("LOG_LEVEL").map(String::as_str), Some("debug"));
            }
            other => panic!("unexpected execution {other:?}"),
        }
    }

    #[test]
    fn wasm_manifest() {
        let manifest = Manifest::parse(
            r#"
api = "tpt.runtime/v1"
[workload]
name = "hello"
[execution]
backend = "wasm"
module = "hello.wasm"
class = "service"
"#,
        )
        .unwrap();
        let spec = manifest.into_workload_spec().unwrap();
        match spec.execution {
            ExecutionSpec::WasmModule(wasm) => {
                assert_eq!(wasm.class, WasmClass::Service);
                assert_eq!(wasm.module, PathBuf::from("hello.wasm"));
            }
            other => panic!("unexpected execution {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_api_version() {
        let err = Manifest::parse(
            r#"
api = "tpt.runtime/v99"
[workload]
name = "x"
[execution]
backend = "wasm"
module = "m.wasm"
"#,
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidConfiguration);
        assert!(err.message.contains("tpt.runtime/v1"));
    }

    #[test]
    fn rejects_unknown_fields_and_missing_backend_fields() {
        assert!(Manifest::parse(
            "api = 'tpt.runtime/v1'\n[workload]\nname='x'\nextra=1\n[execution]\nbackend='wasm'\nmodule='m'"
        )
        .is_err());

        let missing = Manifest::parse(
            "api = 'tpt.runtime/v1'\n[workload]\nname='x'\n[execution]\nbackend='windows'\n",
        )
        .unwrap()
        .into_workload_spec()
        .unwrap_err();
        assert!(missing.message.contains("execution.program"));
    }

    #[test]
    fn rejects_invalid_volume_mode() {
        let err = Manifest::parse(
            r#"
api = "tpt.runtime/v1"
[workload]
name = "x"
[execution]
backend = "windows"
program = "a.exe"
[[volumes]]
name = "v"
mount = "/v"
mode = "append"
"#,
        )
        .unwrap()
        .into_workload_spec()
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidConfiguration);
    }
}
