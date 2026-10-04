# Advanced Runtime — Research Notes (SPEC Phase 9)

Status: research record for the Phase 9 roadmap items. Each section states
what exists today in this runtime, what the realistic mechanism is, and
what blocks it. Nothing here is implemented yet; this document is the
design memory for when those phases open.

## 1. Snapshots

**What exists.** Workload records are in-memory; the daemon restart loses
the registry (processes are reaped by job kill-on-close). Event history and
logs persist on disk.

**Realistic mechanism.** Two distinct "snapshots" are worth separating:

- *Registry snapshot* (cheap, near-term): periodically persist
  `WorkloadInfo` + resolved resources to `<state>/registry.jsonl`. On
  daemon start, mark records whose PIDs are gone as `Failed` ("restart
  reconciliation", already on the workload crate's roadmap) instead of
  dropping them. This gives continuity of history and names after restarts
  without claiming to preserve execution.
- *Execution snapshot* (hard): serialize live process state. For WASM,
  wasmtime offers no store serialization for WASI contexts — a module
  snapshot would have to be built at the *service* level (checkpoint the
  module's own state via an exported protocol), not the engine level. For
  native Windows processes, this is full checkpoint/restore (below).

**Recommendation.** Implement the registry snapshot with restart
reconciliation first; treat execution snapshots as a consequence of
checkpoint/restore progress.

**Status: the registry snapshot is implemented** — the manager persists
after every lifecycle transition (`with_snapshot_path`) and the daemon
reconciles on boot (`reconcile`): mid-flight workloads are attributed as
`Failed (killed)`, `Created` stay startable, terminal records remain as
history with readable logs.

## 2. Checkpoint/restore

**Native Windows processes.** No supported user-mode full-process
checkpoint exists in Windows. Candidate mechanisms, in order of fit:

1. *Job-hierarchical save/restore at the application level* — workloads
   that implement a checkpoint protocol (flush state to a mounted volume,
   exit 0; restart replays). The runtime's role: a `tpt checkpoint`
   command that quiesces (stop with a grace signal), snapshots the
   workload's volumes, and records a restore manifest. Volumes are already
   the state seam (SPEC §15), so this fits the existing model.
2. *Memory-dump based resumption* — `MiniDumpWriteDump` captures state but
   there is no supported in-process restore; research-only.
3. *Virtualization-based* — run the workload in a VM/HCS utility VM and
   snapshot that. This is the honest full-fidelity path and belongs
   alongside the Boxcar isolation provider (SPEC §17): if Boxcar grows a
   VM primitive, checkpoint/restore falls out of it.

**WASM.** Deterministic replay is cheaper than snapshotting: record
(inputs, fuel, seed) and re-run into a trap point. The fuel/epoch
machinery already in place makes replay bounds enforceable.

## 3. Workload migration

Depends on checkpoint/restore. The near-term form is *re-creation
migration*: ship the manifest + volume contents to another daemon and
start there (spec is declarative already; volumes are directories). This
gives "move my project" semantics without live-state migration. Live
migration inherits exactly the checkpoint/restore constraints above.

## 4. Remote runtime

**What exists.** The API is a request/response envelope over a local pipe
(§30); the CLI is already a pure client.

**Path.** The `ApiClient` transport is the seam: a TCP/TLS transport with
the same framing would make `tpt --remote host` work without CLI changes.
What must come with it: authentication (named pipes give Windows ACLs for
free; a network transport needs a token story), and event stream
multiplexing (already per-connection). SPEC §39's model (local runtime
fronting a remote one) fits: the daemon proxies workload creation marked
with a target selector, keeping policy decisions local.

**Status: TCP is implemented (unauthenticated)** — `serve_tcp` /
`ApiClient` over `config.tcp`, `tpt --remote host:port` on the CLI side.
TLS/token authentication is the remaining piece before untrusted
networks; until then the transport is loopback / trusted-network only and
says so in its docs and banner.

## 5. Edge runtime

The MVP assumptions that do not hold on constrained hosts: wasmtime
compile times and multi-GB debug profiles (already mitigated by
`debug = false`), per-workload job objects (fine), the JSONL event sink
(fine), and nvidia-smi-based discovery (absent on edge GPUs — must degrade
quietly, which it does). A dedicated edge profile mostly means: pool the
wasmtime engine, cap event retention, and replace the JSON secret store
with the platform keystore. None require architectural change.

## 6. Advanced isolation

The layered story is already in place: job objects (Windows) → WASI
sandbox (WASM) → Boxcar isolation provider (OCI, pending). The remaining
gaps tracked in the todo — per-process CUDA isolation, WASM socket grants,
native network/filesystem enforcement — all land on the same two
primitives: a Boxcar isolation provider and, for GPU, CUDA MPS or
MIG-style partitioning exposed through the device registry's claim model
(`gpu:<n>` claims are already exclusive-capable).

## 7. Runtime optimization

Measured posture today: zero-copy is *not* claimed anywhere (log capture
copies; WASI stdout pipes copy once into `MemoryOutputPipe`, then again to
disk). The optimization backlog, in expected value order: stream WASI
stdout directly to the log file writer (removes one full copy + the 1 MiB
pipe cap), pool wasmtime instances for `wasm-service` workloads, and
sample job accounting on a timer instead of per-request. Archon shared
buffers (SPEC §16) would replace the remaining log/media copies once that
substrate exists.
