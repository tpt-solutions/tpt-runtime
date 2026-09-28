# tpt-runtime-daemon

[![crate](https://img.shields.io/badge/crate-tpt--runtime--daemon-orange)](https://crates.io/crates/tpt-runtime-daemon)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--daemon-blue)](https://docs.rs/tpt-runtime-daemon)

The Windows host daemon (SPEC §43): builds every subsystem — storage, network,
devices, secrets, events, metrics, and the execution backends — and serves the
local API until shutdown.

This crate is deliberately thin. It is composition, not logic: all the
behaviour lives in the subsystems it wires together.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-daemon = "0.1"
```

The binary is `tpt-runtime-daemon`.

## Running it

```console
cargo build --release -p tpt-runtime-daemon
tpt-runtime-daemon
```

Normally you start it through the CLI instead, which re-invokes this binary
detached:

```console
tpt daemon start
tpt daemon stop
```

Both honour `--state-dir` and `--pipe`, and the `TPT_RUNTIME_DIR` /
`TPT_RUNTIME_PIPE` environment variables.

## What `run` does

`tpt_runtime_daemon::run` is the whole daemon in one call:

```rust,no_run
use tpt_runtime_config::DaemonConfig;

# async fn demo() -> tpt_runtime_core::Result<()> {
// Resolves the state dir and pipe, creates the directory layout,
// discovers GPUs, registers the backends, and serves until shutdown.
tpt_runtime_daemon::run(DaemonConfig::from_env()).await
# }
```

In order, it:

1. calls `prepare_dirs`, creating `logs/`, `volumes/` and the event and secret
   files' parent directories;
2. builds the `EventHub` (with the JSONL sink), `MetricsRegistry`,
   `StorageManager`, `NetworkManager`, `DeviceRegistry` and `SecretStore`;
3. discovers GPUs via `nvidia-smi` and registers them as `gpu:<index>` devices;
4. constructs the `WorkloadManager` with a `PolicyEngine` built from the
   discovered host capacity;
5. registers the Windows, WASM, OCI and Linux backends;
6. serves the local API until `daemon.shutdown` or a signal.

## Process lifetime is tied to workloads

This is the property that makes a daemon crash survivable. Native workloads
run inside Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so when the
daemon dies the OS tears down every workload process with it. A crashed daemon
cannot leave orphaned workload processes behind (SPEC §12, SPEC §45
runtime-restart behaviour).

Event history and logs are persisted to disk, so a restarted daemon can still
report what happened before it died.

## State directory layout

Under `TPT_RUNTIME_DIR` (defaulting to a per-user location):

```text
logs/          per-workload stdout and stderr
volumes/       logical volume backing directories
events.jsonl   the structured event log
secrets.json   the secret store
```

## Testing

```console
cargo test -p tpt-runtime-daemon
```

The daemon is composition, so its behaviour is covered by the integration
tests of the crates it assembles, plus the workload manager's end-to-end tests.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
