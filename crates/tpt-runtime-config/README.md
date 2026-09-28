# tpt-runtime-config

[![crate](https://img.shields.io/badge/crate-tpt--runtime--config-orange)](https://crates.io/crates/tpt-runtime-config)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--config-blue)](https://docs.rs/tpt-runtime-config)

Declarative configuration: the `tpt.runtime/v1` workload manifest parsed from
TOML, plus daemon settings resolved from the environment (SPEC §31, §32, §49).

This is the crate most adopters touch first. It turns a human-editable TOML
document into a validated, backend-independent `WorkloadSpec`.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-config = "0.1"
```

## A complete manifest

```toml
api = "tpt.runtime/v1"

[workload]
name = "example"

[execution]
backend = "windows"
program = "myapp.exe"
args = ["--serve", "--port", "8080"]

[resources]
cpu = 4
memory = "4GiB"
timeout_secs = 300

[network]
mode = "service"

[network.expose]
http = 8080

[[volumes]]
name = "source"
mount = "/workspace"
mode = "read-write"

[[devices]]
id = "gpu:0"
mode = "compute"

[[capabilities]]
name = "network.outbound"

[labels]
team = "platform"
```

Every table is `deny_unknown_fields`: a typo such as `[resouces]` or a
misspelled key is a hard error at parse time rather than a silently ignored
setting.

## Parsing and converting

```rust
use tpt_runtime_config::Manifest;

let toml_text = r#"
api = "tpt.runtime/v1"

[workload]
name = "example"

[execution]
backend = "windows"
program = "myapp.exe"

[resources]
cpu = 2
memory = "512MiB"

[network]
mode = "none"
"#;

let manifest = Manifest::parse(toml_text).unwrap();

// The manifest is backend-independent from here on.
let spec = manifest.into_workload_spec().unwrap();
assert_eq!(spec.name, "example");
assert_eq!(spec.execution.backend(), tpt_runtime_model::BackendKind::Windows);
assert_eq!(spec.resources.memory.unwrap().0, 512 * 1024 * 1024);
```

`Manifest::from_path` is the file equivalent, and reports `NotFound` for a
missing file.

## Versioning

The `api` key pins the manifest schema. Anything other than the supported
version is rejected with a message naming both the requested and the
implemented version, so a stale manifest fails loudly.

```rust
use tpt_runtime_config::{MANIFEST_API_VERSION, Manifest};
use tpt_runtime_core::error::ErrorKind;

assert_eq!(MANIFEST_API_VERSION, "tpt.runtime/v1");

let old = r#"
api = "tpt.runtime/v0"

[workload]
name = "example"

[execution]
backend = "windows"
program = "myapp.exe"
"#;

let err = Manifest::parse(old).unwrap_err();
assert_eq!(err.kind, ErrorKind::InvalidConfiguration);
assert!(err.message.contains("tpt.runtime/v0"));
```

The `api` key defaults to the current version, so a minimal manifest may omit
it.


## Backend selection

The `[execution]` table is a single table whose `backend` key selects which
set of fields apply. This keeps a manifest readable while staying strict about
irrelevant keys.

| `backend` | Fields |
| --- | --- |
| `windows` | `program`, `args`, `working_dir` |
| `linux` | `distro`, `command`, `args` |
| `oci` | `image` |
| `wasm` | `module`, `class` |

A WASM manifest, for example:

```toml
api = "tpt.runtime/v1"

[workload]
name = "wasm-hello"

[execution]
backend = "wasm"
module = "examples/hello.wasm"
class = "command"

[resources]
fuel = 100000
timeout_secs = 30

[network]
mode = "none"
```

## Daemon configuration

`DaemonConfig` resolves runtime state locations and the API pipe name, honouring
two environment overrides:

| Variable | Effect |
| --- | --- |
| `TPT_RUNTIME_DIR` | state directory (volumes, logs, events, secrets) |
| `TPT_RUNTIME_PIPE` | local API pipe name |

```rust
use tpt_runtime_config::{DaemonConfig, DEFAULT_PIPE_NAME};
use std::path::PathBuf;

let config = DaemonConfig {
    state_dir: PathBuf::from("/var/lib/tpt"),
    pipe_name: DEFAULT_PIPE_NAME.to_owned(),
    max_clients: 32,
};

assert_eq!(config.volumes_dir(), PathBuf::from("/var/lib/tpt/volumes"));
assert_eq!(config.logs_dir(), PathBuf::from("/var/lib/tpt/logs"));
assert_eq!(config.events_file(), PathBuf::from("/var/lib/tpt/events.jsonl"));
assert_eq!(config.secrets_file(), PathBuf::from("/var/lib/tpt/secrets.json"));
```

`DaemonConfig::from_env` applies the overrides on top of `default()`.
`prepare_dirs` creates the directory layout and is idempotent.

The default pipe is `\\.\pipe\tpt-runtime-api`.

## Testing

```console
cargo test -p tpt-runtime-config
```

Tests cover valid manifests, version mismatch, unknown-field rejection, volume
and device mode parsing, and the malformed-input negative cases required by
SPEC §45.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
