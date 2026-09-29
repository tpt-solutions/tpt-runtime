# TPT Runtime

A unified workload runtime for Windows and heterogeneous compute, providing a coherent execution, storage, networking, security, device, and observability environment for Windows, Linux, OCI, WASM, and TPT-native workloads.

tpt-runtime is not a WSL clone. Its goal is to provide a common workload substrate beneath Windows and Linux execution rather than making one pretend to be the other — so that native processes, containers, WASM modules, and AI workloads can be managed with the same lifecycle, resource, and capability model.

See [SPEC.md](SPEC.md) for the full design specification, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the implemented architecture, and [todo.md](todo.md) for the project checklist.

## Status

MVP implemented (SPEC §43): Windows host daemon, `tpt.runtime/v1` manifests, workload lifecycle, native Windows-process backend (Job Objects), WASM backend (wasmtime + WASI), logical volumes/networks, capability model, resource accounting, local named-pipe API, `tpt` CLI, structured events and observability. OCI support is prepared (references, bundles, content store) and awaits `tpt-boxcar` isolation primitives; Linux runs through WSL.

Requires a Rust toolchain (stable, MSVC) to build. Windows is the primary host.

## Quick start

```console
cargo build --release -p tpt-runtime-cli -p tpt-runtime-daemon

tpt daemon start            # background daemon (named pipe \\.\pipe\tpt-runtime-api)
tpt status                  # daemon status

tpt run --windows cmd.exe -- /C echo hello
tpt run --manifest examples/windows-echo.toml
tpt run --wasm examples/hello.wasm
tpt list
tpt inspect <workload-id>
tpt logs <workload-id>
tpt stop <workload-id>
tpt events --limit 30

tpt volume create project
tpt devices                 # GPUs discovered via nvidia-smi
tpt daemon stop
```

Project environments (SPEC §34): one `tpt.toml` describes a whole project's
workloads and their dependencies; `tpt up` starts them in dependency order,
`tpt down` stops them in reverse.

```console
tpt init --template services    # scaffold tpt.toml (minimal, services, wasm)
tpt up                          # start every workload, dependencies first
tpt up --only api               # start one workload plus its dependencies
tpt down                        # stop and destroy the project's workloads
```

Every CLI command is a client of the daemon's local API; the wire protocol (newline-delimited JSON over a named pipe) is documented in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#6-api-protocol-30).

## Examples

Runnable manifests live in [examples/](examples/), each annotated with what it demonstrates:

| File | Shows |
| --- | --- |
| `windows-echo.toml` | the smallest useful manifest: one process, no network |
| `windows-service.toml` | service mode, a fixed port, a volume, limits, labels |
| `windows-limited.toml` | over-requests that policy denies or clamps |
| `windows-gpu.toml` | a device request plus the matching capability |
| `windows-secret.toml` | a secret held behind a capability, not an env var |
| `wasm-hello.toml` | a WASM module bounded by fuel and a timeout |

Every example manifest is covered by a test in `tpt-runtime-config`, so the shipped examples cannot drift out of validity.

## Crates

21 crates, layered so the workload model never depends on a backend. Each crate has its own README with runnable examples, and its own changelog.

### Foundations

| Crate | Responsibility |
| --- | --- |
| [`core`](crates/tpt-runtime-core) | identifiers, lifecycle state machine, `RuntimeError`, `RuntimeEvent`, `ResourceUsage` |
| [`model`](crates/tpt-runtime-model) | `WorkloadSpec`, execution payloads, memory/network/device/volume models |
| [`config`](crates/tpt-runtime-config) | `tpt.runtime/v1` TOML manifests, daemon settings |

### Policy and access

| Crate | Responsibility |
| --- | --- |
| [`capability`](crates/tpt-runtime-capability) | typed capability grants, revocation, checks |
| [`policy`](crates/tpt-runtime-policy) | admission: hard/soft limits, GPU presence checks |
| [`security`](crates/tpt-runtime-security) | capability-gated secret store |

### Backends

| Crate | Responsibility |
| --- | --- |
| [`process`](crates/tpt-runtime-process) | the `ExecutionBackend` / `WorkloadInstance` traits, log capture |
| [`windows`](crates/tpt-runtime-windows) | Job Object isolation, kill-tree, env allowlist, job accounting |
| [`wasm`](crates/tpt-runtime-wasm) | wasmtime + WASI p1, fuel/epoch limits, preopened volumes |
| [`oci`](crates/tpt-runtime-oci) | image references, bundle model, content store; start awaits Boxcar |
| [`linux`](crates/tpt-runtime-linux) | WSL-backed execution, layered strategy phase 1 |

### Resources and observability

| Crate | Responsibility |
| --- | --- |
| [`storage`](crates/tpt-runtime-storage) | logical volumes over host directories |
| [`network`](crates/tpt-runtime-network) | network intents and port allocation |
| [`device`](crates/tpt-runtime-device) | logical device registry, claims |
| [`gpu`](crates/tpt-runtime-gpu) | NVIDIA discovery via `nvidia-smi` |
| [`observe`](crates/tpt-runtime-observe) | event hub (broadcast + JSONL), metrics registry |

### Interfaces

| Crate | Responsibility |
| --- | --- |
| [`ipc`](crates/tpt-runtime-ipc) | request/response envelope, NDJSON framing |
| [`api`](crates/tpt-runtime-api) | named-pipe server, CLI client |
| [`workload`](crates/tpt-runtime-workload) | lifecycle orchestration across backends |
| [`daemon`](crates/tpt-runtime-daemon) | assembles the stack, serves until shutdown |
| [`cli`](crates/tpt-runtime-cli) | the `tpt` binary; a pure API client |

## Development

```console
cargo build --workspace
cargo test --workspace
cargo test -p tpt-runtime-config   # also validates every example manifest
```

## Related projects

- `tpt-archon` — low-level substrate (storage, IPC, resources)
- `tpt-boxcar` — workload isolation, packaging, networking, sandboxing
- `tpt-infer` — inference
- `tpt-dsp` — signal processing
- `tpt-orchestra` — multi-agent / repository orchestration

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

Copyright © 2026 TPT Solutions.
