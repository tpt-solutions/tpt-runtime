# TPT Runtime — Project Checklist

Tracks work against [SPEC.md](SPEC.md). Organized by the spec's Phase Roadmap (§44), plus the MVP target (§43) and cross-cutting testing/security work (§45–46).

Status after the first implementation pass: the MVP (§43) is working end to
end on Windows — daemon, CLI, Windows-process and WASM backends, lifecycle,
events, observability — with 50 test suites (159 tests) green. See
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
- [x] WSL integration *(verified against a real Ubuntu-24.04 distro: env + volume paths cross via WSLENV `/p` translation, stop terminates the relay, sanitized host env; adaptive tests skip gracefully without distros; per-workload in-distro accounting pending)*
- [x] Linux workload lifecycle *(create → start → exit / stop verified through the manager path against a real distro)*
- [ ] Linux networking *(manager-level intents resolve; in-distro enforcement awaits the Boxcar isolation layer)*
- [x] Linux storage *(granted volumes arrive as `TPT_VOLUME_<NAME>` env vars with WSLENV path translation; dedicated distro filesystems pending)*
- [ ] Linux observability *(relay I/O accounting only; per-workload /proc metrics pending)*
- [x] Capability translation *(manifest env and granted mounts cross the WSL boundary through WSLENV; the host environment itself never leaks)*

## Phase 7 — GPU

- [x] GPU discovery *(nvidia-smi; verified on RTX 3050 host)*
- [ ] GPU capability model *(device registry + policy checks exist; per-process CUDA isolation pending)*
- [ ] NVIDIA integration
- [x] GPU telemetry *(nvidia-smi sampling into a latest-value cache, refreshed every 5s by the daemon; surfaced through `daemon.status` and `tpt status`)*
- [ ] TPT Infer integration
- [ ] GPU resource policies

## Phase 8 — Developer Platform

- [x] Project environments *(`tpt.toml` with `[project]` + `[[workload]]` entries: external manifest paths or inline manifest bodies; project name tagged as the `tpt.project` label)*
- [x] `tpt up` *(create + start in dependency order; `--only` pulls in transitive dependencies; compose-style failure keeps earlier workloads running)*
- [x] `tpt down` *(stop + destroy every workload labeled with the project, reverse dependency order, tolerant of already-exited workloads)*
- [x] Service dependencies *(`depends_on` with cycle detection, topological start ordering, reverse stop ordering)*
- [ ] VS Code integration *(extension is a separate deliverable; the API + labels groundwork it needs is in place)*
- [x] Project templates *(embedded `tpt init` templates: minimal, services, wasm — each template is tested to parse and materialize)*

## Phase 9 — Advanced Runtime

- [ ] Snapshots
- [x] Checkpoint/restore research *(docs/RESEARCH-ADVANCED-RUNTIME.md: volume-checkpoint protocol, WASM deterministic replay, VM-based path via Boxcar)*
- [ ] Remote runtime
- [ ] Edge runtime
- [x] Workload migration research *(docs/RESEARCH-ADVANCED-RUNTIME.md: re-creation migration via declarative manifest + volumes first; live migration inherits checkpoint/restore constraints)*
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
- [x] runtime → OCI *(store/bundle/backend plus the malicious-image suite; `start` itself awaits Boxcar)*
- [ ] runtime → Archon
- [ ] runtime → Boxcar

### Compatibility tests
- [ ] Windows
- [x] Linux *(real WSL distro tests for lifecycle, env/volume translation and relay stop; adaptive skip on hosts without distros)*
- [ ] OCI
- [ ] WASM
- [ ] GPU
- [ ] Filesystem
- [ ] Network

### Failure tests
- [x] Workload crashes *(watcher → `workload.failed`, killed-vs-crash attribution)*
- [x] Resource exhaustion *(WASM fuel + adversarial memory growth; OS-level commit limit via job objects with an unlimited control workload)*
- [x] Capability denial *(unit level; WASM backend enforcement covered in the adversarial suite — native fs/net stays policy+audit by design)*
- [x] Device disappearance *(unplug → attach denial for new workloads, re-plug restores, existing claims untouched)*
- [x] Network failure *(static port conflicts, explicit failure when no ephemeral port is pickable, expose-mode validation)*
- [x] Storage failure *(missing/deleted volume backing dirs, corrupt `volume.json` fails open loudly, non-empty removal refusal)*
- [x] Runtime restart *(scripted: dropping the last job handle reaps the workload without an explicit stop)*
- [ ] Host restart

## Security Testing (§46)

- [x] Capability escalation tests *(grants are exactly the manifest set; rw mounts add no filesystem.write; unknown grants carried for audit; missing secret/GPU/overcommit denied at create)*
- [x] Filesystem escape tests *(WASI preopen escape attempts + fd least-privilege + read-only enforcement; bundle entry-point traversal incl. Windows-style paths; volume name validation)*
- [x] Process isolation tests *(per-workload job objects: stopping one workload leaves siblings running to a clean exit; env is sanitized, not inherited)*
- [x] Network isolation tests *(deny-by-default intents with no network events; port conflict + release lifecycle; WASM exposes no fds beyond granted preopens)*
- [x] Secret leakage tests *(e2e: value absent from inspect, list, events and workload logs; resolution capability-gated; unknown secret denies create)*
- [x] Device access tests *(exclusive conflict at create, shared compute allowed, claims released on settle/destroy, policy denies undiscovered GPU indices)*
- [x] Malformed manifest tests
- [x] Malicious image tests *(poisoned tag files cannot traverse the store, malformed digests rejected, rootfs-less blobs unresolved, hostile entry points incl. Windows paths)*
- [x] WASM sandbox tests *(adversarial suite: manifest-scoped env, preopen escape, fuel-bounded memory growth, foreign import refusal, wall-clock kill)*
- [ ] OCI isolation tests *(store/bundle containment covered; workload isolation boundary awaits Boxcar)*
