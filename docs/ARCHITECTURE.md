# tpt-runtime — Architecture

Status: implemented through the MVP (SPEC §43). This document records the
crate boundaries (Phase 0), the runtime data flow, and the integration
points reserved for `tpt-archon` and `tpt-boxcar`.

## 1. Crate boundaries

The workspace is organized so that the workload model never depends on a
backend, and backends never depend on each other:

```text
                      tpt (CLI)                tpt-runtime-daemon
                          │                            │
                          │        local API (named pipe, NDJSON)
                          │                            │
                          └────────► tpt-runtime-api ◄─┘
                                         │
                                 tpt-runtime-workload   ← the only crate that
                                         │                knows all backends
        ┌──────────────┬─────────────────┼──────────────────┬──────────────┐
tpt-runtime-windows  tpt-runtime-wasm  tpt-runtime-oci   tpt-runtime-linux  (future)
        └──────────────┴─────────────────┼──────────────────┴──────────────┘
                                         │
                            tpt-runtime-process (backend trait)
                                         │
   ┌─────────┬─────────┬─────────┬───────┴────┬──────────┬───────────┐
tpt-runtime-storage  network  device  capability  policy  security
   └─────────┴─────────┴───────┬───┴────────────┴──────────┴───────────┘
                               │
              tpt-runtime-model  (workload model, SPEC §9)
                               │
              tpt-runtime-core  (ids, states, errors, events, usage)
                               │
              tpt-runtime-config (manifests tpt.runtime/v1, daemon config)
```

Supporting crates:

| Crate | Responsibility (SPEC section) |
| --- | --- |
| `tpt-runtime-core` | identifiers, lifecycle state machine, `RuntimeError`, `RuntimeEvent`, `ResourceUsage` (§8, §26, §28) |
| `tpt-runtime-model` | `WorkloadSpec`, execution payloads, memory/network/device/volume models (§9, §18, §19, §25) |
| `tpt-runtime-config` | `tpt.runtime/v1` TOML manifests, project environments (`tpt.toml`), daemon settings (§31–§34, §49) |
| `tpt-runtime-capability` | typed capability grants, revocation, checks (§5.2, §23) |
| `tpt-runtime-policy` | admission: hard/soft limits, GPU presence checks (§25) |
| `tpt-runtime-process` | `ExecutionBackend` / `WorkloadInstance` traits, log capture (§10) |
| `tpt-runtime-windows` | Job Object isolation, kill-tree, env allowlist, job accounting (§12) |
| `tpt-runtime-wasm` | wasmtime + WASI p1, fuel/epoch limits, preopened volumes (§14) |
| `tpt-runtime-oci` | image references, bundle model, content store; start awaits Boxcar (§13) |
| `tpt-runtime-linux` | WSL-backed execution, layered strategy phase 1 (§11) |
| `tpt-runtime-storage` | logical volumes over host directories (§15) |
| `tpt-runtime-network` | network intents and port allocation (§18) |
| `tpt-runtime-device` | logical device registry, claims (§19) |
| `tpt-runtime-gpu` | NVIDIA discovery and telemetry via `nvidia-smi` (§20) |
| `tpt-runtime-ipc` | request/response envelope, NDJSON framing (§22, §30) |
| `tpt-runtime-security` | capability-gated secret store (§24) |
| `tpt-runtime-observe` | event hub (broadcast + JSONL), metrics registry (§27, §28) |
| `tpt-runtime-api` | named-pipe server, CLI client (§30) |
| `tpt-runtime-workload` | lifecycle orchestration across backends (§26) |
| `tpt-runtime-daemon` | assembles the stack, serves until shutdown (§43) |
| `tpt-runtime-cli` | `tpt` binary; pure API client (§29, §30) |

Rules enforced by review:

- `core` and `model` must stay platform-independent.
- Only `tpt-runtime-windows` (and later GPU/device integration) contains
  `unsafe`; all FFI is confined to `windows/src/backend/job.rs`.
- Only `tpt-runtime-workload` may depend on concrete backends.
- The CLI must not embed runtime logic; it speaks the API protocol only.

## 2. Lifecycle flow

`workloads.create` and `workloads.start` implement SPEC §3.1:

```text
manifest (TOML) ─► WorkloadSpec ─► validate ─► policy admission
    ─► resolve volumes (storage) ─► assign network (ports)
    ─► check devices ─► check secrets ─► backend.prepare()
    = Created

start ─► backend.start(spec, StartContext) ─► WorkloadInstance
    ─► events: capability.granted / network.connected / workload.started
    ─► watcher thread: instance.exit() ─► final stats ─► release ports
    ─► state Stopped|Failed ─► workload.stopped|failed event
```

The state machine itself (which transitions are legal) lives in
`tpt-runtime-core::state` and is unit-tested against the SPEC §3.1 path.

## 3. Isolation and security posture (MVP)

- **Windows processes**: one Job Object per workload with
  `KILL_ON_JOB_CLOSE` (daemon crash cannot leak workloads), optional
  `PROCESS_MEMORY` limit, job-wide termination for stop/kill, and a
  sanitized environment allowlist — the daemon never leaks its own env.
- **WASM**: deny-by-default WASI. Only volumes from capability-backed
  mounts are preopened (`FsPerms::ReadOnly` when mounted read-only), env is
  manifest-scoped, and no sockets are granted. CPU is bounded by fuel,
  wall-clock by epoch interruption, and stop is honored within ~10 ms even
  inside a busy loop (epoch deadline callback).
- **Secrets** never enter environment variables by default; values are
  released only against the matching `secret:<name>` capability, and list
  endpoints expose names only.
- **Device claims** follow the workload lifecycle: exclusive conflicts deny
  attach (create) and re-attach (start) loudly, and claims are released when
  a workload settles or is destroyed.
- **Denial is loud** (SPEC §48): missing GPU, unknown volume, unsatisfiable
  limits, and unregistered backends are errors with attribution
  (`workload → backend → operation`), never silent downgrades.

Known MVP limitations (documented, not hidden):

- Windows native processes cannot be denied host resources beyond memory
  limits; network/filesystem denial for native processes is policy +
  audit for now. WASM carries the enforced story.
- Read-only volume enforcement for native processes is advisory; WASI
  preopens enforce it for real.
- Workload records are in-memory; a daemon restart loses the registry
  (workload processes are reaped by kill-on-close). Event history and
  logs persist on disk.
- `pause`/`resume` report `not_implemented` until a backend supports
  suspension.
- OCI `start` requires the `tpt-boxcar` isolation provider (below).

## 4. tpt-archon integration points (SPEC §16)

Archon is the substrate; the runtime keeps it swappable:

1. **Storage** — `tpt-runtime-storage::StorageManager` is the seam.
   Directory-backed `Volume` today; an Archon-backed store implements the
   same `create/get/mount_path` surface with page-cache and dedup behind it.
2. **IPC** — `tpt-runtime-ipc` defines the envelope/framing; an Archon
   transport (shared-memory channels) can replace the named-pipe byte
   stream without touching API semantics.
3. **Zero-copy buffers** — `WorkloadInstance::stats` and log capture are
   the current copy points; Archon shared buffers would replace log and
   media paths (Phase 5), starting with the WASM stdout pipe.
4. **Resource accounting** — job-object sampling feeds
   `tpt-runtime-observe::MetricsRegistry`; an Archon accounting feed would
   push the same `ResourceUsage` records.

## 5. tpt-boxcar integration points (SPEC §17)

Boxcar provides workload-level isolation and packaging:

1. **OCI start** — `tpt-runtime-oci::OciBackend::start` is the reserved
   seam: it prepares a validated `Bundle` and awaits a Boxcar
   "run this bundle isolated" primitive (image pull, layer unpack, and
   the isolation boundary).
2. **WASM sandbox/service** — the WASM backend's builder code
   (preopens, limits) is intended to migrate into a shared Boxcar WASM
   service layer so plugins across TPT projects get identical semantics.
3. **Service networking** — `NetworkMode::Service` currently allocates
   host ports; Boxcar's service mesh would take over the resolution while
   the manifest intent stays identical.
4. **Secrets at rest** — the MVP JSON store is the fallback; a Boxcar
   secret facility (DPAPI/TPM-backed) replaces `tpt-runtime-security`'s
   persistence without changing the capability gate.

## 6. API protocol (§30)

Transport: `\\.\pipe\tpt-runtime-api` (override: `TPT_RUNTIME_PIPE`).
Framing: newline-delimited UTF-8 JSON.

- Request: `{"id": 1, "method": "workloads.list", "params": {...}}`
- Response: `{"id": 1, "result": ...}` or `{"id": 1, "error": {kind, message, ...}}`
- Events (server → client, no `id`): `{"event": "workload.started", ...}`
  after `events.subscribe`.

Methods: `daemon.status`, `daemon.shutdown`, `workloads.{create,start,
stop,restart,pause,destroy,list,inspect,logs,usage}`, `events.{history,
subscribe}`, `volumes.{create,list,remove}`, `devices.list`,
`secrets.{set,list,delete}`.

## 7. Testing map (SPEC §45–§46)

| Area | Where |
| --- | --- |
| State transitions | `core/src/state.rs` tests |
| Manifests | `config/src/manifest.rs` tests |
| Capabilities | `capability/src/capability.rs` tests |
| Policies | `policy/src/policy.rs` tests |
| Resource calculations | `core/src/usage.rs`, `model/src/resources.rs` tests |
| Identifiers / events | `core/src/{id,event}.rs` tests |
| runtime → Windows process | `windows` unit tests + `workload/tests/manager_e2e.rs` |
| runtime → WASM | `wasm/tests/wasi.rs` (hello, fuel, stop, validation) |
| runtime → OCI | `oci` store/bundle/backend tests + `oci/tests/malicious_image.rs` (start = explicit pending) |
| Failure: workload crashes | watcher → `workload.failed` (e2e) |
| Failure: runtime restart | `windows` job tests: dropping the last job handle reaps the workload |
| Failure: resource exhaustion | WASM fuel + adversarial growth (`wasm/tests/adversarial.rs`); OS-level commit limit (`windows` job tests) |
| Failure: device disappearance | `workload/tests/security_isolation.rs` (unplug → attach denial, re-plug) |
| Failure: network failure | port conflicts and allocation guard (`workload/tests/security_isolation.rs`, `network` unit tests) |
| Failure: storage failure | missing/deleted volumes, corrupt metadata (`workload/tests/security_isolation.rs`, `storage` unit tests) |
| Capability denial | `capability` require/check tests |
| Security: capability escalation | grants are exactly the manifest set (`workload/tests/security_isolation.rs`) |
| Security: filesystem escape | WASI preopen escape + read-only enforcement (`wasm/tests/adversarial.rs`); bundle entry-point traversal (`oci/tests/malicious_image.rs`) |
| Security: process isolation | per-workload job objects; stopping one leaves siblings intact |
| Security: network isolation | deny-by-default intents; WASM fd least-privilege; port lifecycle |
| Security: secret leakage | value absent from inspect/list/events/logs e2e; capability-gated resolve |
| Security: device access | exclusive conflicts, availability, policy denial of absent GPUs |
| Security: WASM sandbox | `wasm/tests/adversarial.rs` (env scoping, escape, growth, foreign imports, wall-clock kill) |
| Developer platform | `tpt up`/`down` over a real pipe (`cli` project_ops e2e); project parsing (`config` project tests) |
| Linux → WSL | real-distro tests in `linux/src/backend.rs` (env + volume translation, relay stop; adaptive skip without distros) |
| GPU telemetry | sampling + cache (`gpu/src/telemetry.rs`); surfaced through `daemon.status` |
| Malformed manifests | `config` negative tests |

## 8. Developer platform (§33–§34)

`tpt.toml` (parsed by `tpt-runtime-config::project`) declares a project's
workloads — each referencing an external `tpt.runtime/v1` manifest or an
inline manifest body — plus `depends_on` edges. The CLI stays a pure API
client: `tpt up` materializes each manifest (the project entry's name is
authoritative, and the workload is labeled `tpt.project = <name>`), then
creates and starts them in dependency order through the daemon;
`tpt down` stops and destroys everything carrying the project's label in
reverse order. `tpt init` scaffolds a project from built-in templates
(minimal, services, wasm). Workload surfaces already built on this:
duplicate-name protection, port allocation and release, and the labels
field on `WorkloadInfo`.
