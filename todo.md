# TPT Runtime — Project Checklist

Tracks work against [SPEC.md](SPEC.md). Organized by the spec's Phase Roadmap (§44), plus the MVP target (§43) and cross-cutting testing/security work (§45–46).

## MVP Capabilities (§43)

- [ ] Windows host daemon
- [ ] Workload manifest
- [ ] Workload lifecycle
- [ ] Native Windows process backend
- [ ] OCI backend through Boxcar-compatible primitives
- [ ] WASM backend
- [ ] Logical volume abstraction
- [ ] Logical network abstraction
- [ ] Capability model
- [ ] Resource accounting
- [ ] CLI
- [ ] Local API
- [ ] Structured events
- [ ] Basic observability

## Phase 0 — Architecture

- [ ] Define workload model
- [ ] Define resource model
- [ ] Define capability model
- [ ] Define lifecycle
- [ ] Define API
- [ ] Define event model
- [ ] Define crate boundaries
- [ ] Document Archon integration points
- [ ] Document Boxcar integration points

## Phase 1 — Runtime Core

- [ ] Create workspace
- [ ] Implement identifiers
- [ ] Implement workload state machine
- [ ] Implement manifests
- [ ] Implement configuration
- [ ] Implement errors
- [ ] Implement events

## Phase 2 — Windows Runtime

- [ ] Runtime daemon
- [ ] Windows process backend
- [ ] Process lifecycle
- [ ] Environment management
- [ ] stdout/stderr capture
- [ ] Resource accounting
- [ ] CLI

## Phase 3 — WASM

- [ ] WASM backend
- [ ] Sandbox
- [ ] Resource limits
- [ ] Filesystem capabilities
- [ ] Network capabilities
- [ ] Service lifecycle

## Phase 4 — OCI

- [ ] OCI image support
- [ ] Image cache
- [ ] Filesystem preparation
- [ ] Workload lifecycle
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

- [ ] Linux backend abstraction
- [ ] WSL integration
- [ ] Linux workload lifecycle
- [ ] Linux networking
- [ ] Linux storage
- [ ] Linux observability
- [ ] Capability translation

## Phase 7 — GPU

- [ ] GPU discovery
- [ ] GPU capability model
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
- [ ] Manifests
- [ ] State transitions
- [ ] Policies
- [ ] Capabilities
- [ ] Resource calculations
- [ ] Identifiers
- [ ] Event serialization

### Integration tests
- [ ] runtime → Windows process
- [ ] runtime → WASM
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
- [ ] Workload crashes
- [ ] Resource exhaustion
- [ ] Capability denial
- [ ] Device disappearance
- [ ] Network failure
- [ ] Storage failure
- [ ] Runtime restart
- [ ] Host restart

## Security Testing (§46)

- [ ] Capability escalation tests
- [ ] Filesystem escape tests
- [ ] Process isolation tests
- [ ] Network isolation tests
- [ ] Secret leakage tests
- [ ] Device access tests
- [ ] Malformed manifest tests
- [ ] Malicious image tests
- [ ] WASM sandbox tests
- [ ] OCI isolation tests
