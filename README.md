# TPT Runtime

A unified workload runtime for Windows and heterogeneous compute, providing a coherent execution, storage, networking, security, device, and observability environment for Windows, Linux, OCI, WASM, and TPT-native workloads.

tpt-runtime is not a WSL clone. Its goal is to provide a common workload substrate beneath Windows and Linux execution rather than making one pretend to be the other — so that native processes, containers, WASM modules, and AI workloads can be managed with the same lifecycle, resource, and capability model.

See [SPEC.md](SPEC.md) for the full design specification, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the implemented architecture, and [todo.md](todo.md) for the project checklist.

## Status

MVP implemented (SPEC §43): Windows host daemon, `tpt.runtime/v1` manifests, workload lifecycle, native Windows-process backend (Job Objects), WASM backend (wasmtime + WASI), logical volumes/networks, capability model, resource accounting, local named-pipe API, `tpt` CLI, structured events and observability. OCI support is prepared (references, bundles, content store) and awaits `tpt-boxcar` isolation primitives; Linux runs through WSL.

Requires a Rust toolchain (stable, MSVC) to build. Windows is the primary host.

## Quick start

```text
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

Every CLI command is a client of the daemon's local API; the wire protocol (newline-delimited JSON over a named pipe) is documented in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#6-api-protocol-30).

## Workspace

21 crates, layered so the workload model never depends on a backend:

`core` (ids, states, errors, events) · `model` (workload model) · `config` (manifests) · `capability` · `policy` · `process` (backend trait) · `windows` · `wasm` · `oci` · `linux` · `storage` · `network` · `device` · `gpu` · `ipc` · `security` · `observe` · `api` · `workload` (manager) · `daemon` · `cli`.

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
