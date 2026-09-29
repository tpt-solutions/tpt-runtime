# TPT Runtime — Project Checklist

Tracks work against [SPEC.md](SPEC.md). Organized by the spec's Phase Roadmap (§44), plus the MVP target (§43) and cross-cutting testing/security work (§45–46).

Status after the first implementation pass: the MVP (§43) is working end to
end on Windows — daemon, CLI, Windows-process and WASM backends, lifecycle,
events, observability — with 46 test suites green. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the map and known MVP
limitations.

Since then: every crate has a README, CHANGELOG and crates.io metadata,
runnable example manifests live in [examples/](examples/), and the workspace
builds with zero clippy warnings.

## Packaging & Hygiene

- [x] Per-crate README and CHANGELOG
- [x] crates.io metadata on every crate
- [x] Runnable example manifests (`examples/`) with a test that parses them
- [x] Zero clippy warnings across the workspace
- [x] CI (build, test, clippy, fmt on Windows and Linux) *(GitHub Actions: fmt, clippy `-D warnings`, test, release build, docs — each on `ubuntu-latest` and `windows-latest`)*
- [ ] Publish crates to crates.io (dependency order)
- [ ] Tagged release / version 0.1.0

## MVP Capabilities (§43)

- [x] Windows host daemon
- [x] Workload manifest
- [x] Workload lifecycle
- [x] Native Windows process backend
- [ ] OCI backend through Boxcar-compatible primitives *(refs, bundle model and content store done; `start` awaits the tpt-boxcar isolation provider)*
- [x] WASM backend
- [x] Logical volume abstraction
- [x] Logical network abstraction *(intents + port allocation; enforcement for native processes is policy/audit — WASM is enforced)*
- [x] Capability model
- [x] Resource accounting
- [x] CLI
- [x] Local API
- [x] Structured events
- [x] Basic observability

## Phase 0 — Architecture

- [x] Define workload model
- [x] Define resource model
- [x] Define capability model
- [x] Define lifecycle
- [x] Define API
- [x] Define event model
- [x] Define crate boundaries
- [x] Document Archon integration points
- [x] Document Boxcar integration points

## Phase 1 — Runtime Core

- [x] Create workspace
- [x] Implement identifiers
- [x] Implement workload state machine
- [x] Implement manifests
- [x] Implement configuration
- [x] Implement errors
- [x] Implement events

## Phase 2 — Windows Runtime

- [x] Runtime daemon
- [x] Windows process backend
- [x] Process lifecycle
- [x] Environment management
- [x] stdout/stderr capture
- [x] Resource accounting
- [x] CLI

## Phase 3 — WASM

- [x] WASM backend
- [x] Sandbox
- [x] Resource limits
- [x] Filesystem capabilities
- [x] Network capabilities *(deny-by-default; socket grants reserved for a future host extension)*
- [x] Service lifecycle *(long-running modules run under epoch/fuel limits and honor stop; service mesh integration later)*

## Phase 4 — OCI

- [ ] OCI image support *(reference parsing, bundle model and content store done; registry pull awaits Boxcar)*
- [ ] Image cache *(local content-addressed store exists; pull/pin policies pending)*
- [ ] Filesystem preparation
- [ ] Workload lifecycle *(start reports `not_implemented` pending Boxcar)*
- [ ] Networking
- [ ] Volumes
- [ ] Capability integration

## Phase 5 — Archon

- [ ] Archon storage adapter
- [ ] Archon IPC adapter
- [ ] Shared buffer support
- [ ] Capability integration
- [ ] Resource accounting integration
- [ ] Zero-copy paths where justified

## Phase 6 — Linux

- [x] Linux backend abstraction *(WSL-backed backend; tested on the error path, real-distro testing pending)*
- [ ] WSL integration *(wsl.exe execution implemented; distro lifecycle, per-workload accounting pending)*
- [ ] Linux workload lifecycle
- [ ] Linux networking
- [ ] Linux storage
- [ ] Linux observability
- [ ] Capability translation

## Phase 7 — GPU

- [x] GPU discovery *(nvidia-smi; verified on RTX 3050 host)*
- [ ] GPU capability model *(device registry + policy checks exist; per-process CUDA isolation pending)*
- [ ] NVIDIA integration
- [ ] GPU telemetry
- [ ] TPT Infer integration
- [ ] GPU resource policies

## Phase 8 — Developer Platform

- [ ] Project environments
- [ ] `tpt up`
- [ ] `tpt down`
- [ ] Service dependencies
- [ ] VS Code integration
- [ ] Project templates

## Phase 9 — Advanced Runtime

- [ ] Snapshots
- [ ] Checkpoint/restore research
- [ ] Remote runtime
- [ ] Edge runtime
- [ ] Workload migration research
- [ ] Advanced isolation
- [ ] Runtime optimization

## Testing Strategy (§45)

### Unit tests
- [x] Manifests
- [x] State transitions
- [x] Policies
- [x] Capabilities
- [x] Resource calculations
- [x] Identifiers
- [x] Event serialization

### Integration tests
- [x] runtime → Windows process
- [x] runtime → WASM
- [ ] runtime → OCI
- [ ] runtime → Archon
- [ ] runtime → Boxcar

### Compatibility tests
- [ ] Windows
- [ ] Linux
- [ ] OCI
- [ ] WASM
- [ ] GPU
- [ ] Filesystem
- [ ] Network

### Failure tests
- [x] Workload crashes *(watcher → `workload.failed`, killed-vs-crash attribution)*
- [ ] Resource exhaustion *(WASM fuel exhaustion covered in wasm tests; OS-level limits pending)*
- [x] Capability denial *(unit level; backend enforcement tests pending)*
- [ ] Device disappearance
- [ ] Network failure
- [ ] Storage failure
- [ ] Runtime restart *(kill-on-close job semantics design-verified; scripted test pending)*
- [ ] Host restart

## Security Testing (§46)

- [ ] Capability escalation tests
- [ ] Filesystem escape tests *(bundle rootfs escape checks exist; runtime-level tests pending)*
- [ ] Process isolation tests
- [ ] Network isolation tests
- [ ] Secret leakage tests *(listing redaction covered in security tests; e2e pending)*
- [ ] Device access tests
- [x] Malformed manifest tests
- [ ] Malicious image tests
- [ ] WASM sandbox tests *(sandbox mechanics tested; adversarial module suite pending)*
- [ ] OCI isolation tests
