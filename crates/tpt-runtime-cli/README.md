# tpt-runtime-cli

[![crate](https://img.shields.io/badge/crate-tpt--runtime--cli-orange)](https://crates.io/crates/tpt-runtime-cli)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--cli-blue)](https://docs.rs/tpt-runtime-cli)

`tpt`: the developer CLI (SPEC §29).

The CLI is a **pure client of the runtime API**. It holds no runtime state of
its own — every command is a request to the daemon. That is why `tpt` works the
same whether the runtime is on the same machine or reachable over a different
transport later.

The binary is named `tpt`.

## Install

```console
cargo build --release -p tpt-runtime-cli -p tpt-runtime-daemon
```

## Quick start

```console
tpt daemon start                     # start the background daemon
tpt status                           # daemon status

tpt run --windows cmd.exe -- /C echo hello
tpt run --manifest examples/windows-echo.toml
tpt run --wasm examples/hello.wasm

tpt list
tpt inspect <workload-id>
tpt logs <workload-id>
tpt stop <workload-id>
tpt events --limit 30

tpt volume create project
tpt devices                           # GPUs discovered via nvidia-smi
tpt daemon stop
```

## Commands

### Daemon

| Command | Description |
| --- | --- |
| `tpt daemon start` | start the daemon detached |
| `tpt daemon stop` | stop the daemon |
| `tpt status` | daemon status summary |

`tpt daemon start` re-invokes the binary with a hidden worker argument, so the
daemon outlives the shell that started it.

### Running workloads

| Command | Description |
| --- | --- |
| `tpt run --windows <EXE> -- <ARGS>` | run a native Windows executable |
| `tpt run --wasm <MODULE>` | run a WASM module (`.wasm` or WAT text) |
| `tpt run --manifest <FILE>` | run a `tpt.runtime/v1` manifest |

Resource flags override the generated manifest, so a quick run and a committed
manifest share one code path:

```console
tpt run --windows myapp.exe -- --serve --cpu 2 --memory 1GiB --timeout 60
```

Everything after `--` is passed to the workload's own command line; the
runtime's own options come before it.

### Inspecting

| Command | Description |
| --- | --- |
| `tpt list` | list workloads |
| `tpt inspect <id>` | full detail for one workload |
| `tpt logs <id>` | stdout, `--stderr` for stderr, `--tail N` to bound |
| `tpt events --limit N` | recent events |
| `tpt events --follow` | live stream until interrupted |

`tpt inspect` and `tpt logs` accept a workload id **or** a workload name.

### Volumes, devices and secrets

| Command | Description |
| --- | --- |
| `tpt volume create <name>` | create a volume |
| `tpt volume list` | list volumes |
| `tpt volume remove <name>` | remove an empty volume |
| `tpt devices` | devices known to the runtime |
| `tpt secret set <name>` | set a secret, value read from stdin |
| `tpt secret list` | list secret names, never values |
| `tpt secret delete <name>` | delete a secret |

`tpt secret set` reads the value from stdin so it never lands in shell history
or in process arguments.

## Globals

| Command | Description |
| --- | --- |
| `tpt stop <id>` | stop a workload |
| `tpt restart <id>` | restart a workload |
| `tpt destroy <id>` | remove a finished workload's records |

## Exit codes and errors

Errors are reported with the daemon's typed `ErrorKind`, so a missing workload
reads differently from a denied capability. A CLI that cannot reach the daemon
says so explicitly and points at `tpt daemon start`, rather than hanging.

## Testing

```console
cargo test -p tpt-runtime-cli
```

The CLI is a thin client; behaviour is covered by the API and workload manager
tests, with the binary exercised end to end through the manual walkthrough in
the repository README.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
