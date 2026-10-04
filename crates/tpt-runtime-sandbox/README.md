# tpt-runtime-sandbox

The runtime's own OCI isolation provider (SPEC §13, §17): the piece that
turns a prepared `Bundle` into a running, isolated Windows process.

## Why this crate exists

SPEC §17 designated `tpt-boxcar` as the isolation provider, leaving
`tpt-runtime-oci::start` returning `not_implemented` until it landed. Boxcar's
`origin` sandbox cannot fill that role today:

- `type: oci` services are documented as **bookkeeping only** — "the manifest
  is parsed and tracked, but no real containerd integration exists yet, so
  nothing is actually spawned". Same for `type: wasm`.
- No Windows isolation: no HCS, no Job Objects, no restricted tokens.
- `tpt-origin` is not published on crates.io; using it means vendoring a
  monorepo that also contains a Go workspace and a Tauri GUI.
- The service mesh that would back `NetworkMode::Service` (Frontier) is a
  Rust proxy behind a Go xDS control plane — an external service, not an
  embeddable library.

Waiting on Boxcar meant waiting on nothing, so the runtime supplies the
boundary itself. It sits behind a trait, so Boxcar can still be dropped in
later without touching `tpt-runtime-oci`.

## What it enforces

| Mechanism | Effect |
| --- | --- |
| Job Object | Kill-on-close ties workload lifetime to the daemon's; memory limits are enforced; CPU/memory/IO accounting is read back from the job. |
| `ActiveProcessLimit` | Caps concurrent processes per workload, bounding fork bombs. |
| Restricted token | `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` — a workload inherits no privilege the daemon holds. |
| Handle inheritance list | Only the two log pipes cross into the child; nothing else the daemon has open is reachable. |
| Sanitized environment | The daemon's environment never leaks in; the workload gets a minimal Windows environment plus the image's and the manifest's variables. |
| Rootfs-scoped entry point | `argv[0]` is re-validated where it becomes a host path, so a rootfs escape fails before any spawn. |
| Mount validation | Every granted volume must exist and be backed by `filesystem.read` (plus `filesystem.write` for read-write mounts). A read-write mount without that grant is refused, not silently downgraded. |
| Network validation | Ports granted to a mode that cannot accept inbound traffic, or an outbound mode without `network.outbound`, are refused. |

The spawn sequence matters and is deliberately ordered:

1. create two inheritable pipes for stdout/stderr;
2. derive a restricted token;
3. describe **only** those two write handles in a
   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`;
4. `CreateProcessAsUserW` with `CREATE_SUSPENDED` — the child does not run yet;
5. assign the still-suspended child to the job object;
6. `ResumeThread`.

Resuming before step 5 is the classic bug here: a process that runs before it
is in a job can already have children outside it.

## What it does *not* do

**This is process-level isolation, not a container filesystem.** Windows has
no `chroot`, so the rootfs is a *source tree* the entry point is resolved
from, not a jail — an image can still read the host filesystem. Genuine
namespace-level confinement needs Hyper-V or WSL and is out of scope here.

Two further constraints follow from the above:

- **Linux images cannot run.** `tpt-runtime-oci` resolves `linux/amd64` by
  default, so `postgres:16` unpacks to an ELF rootfs, and Windows has no ELF
  loader. `bundle_env::plan` sniffs the executable's magic and refuses with an
  error naming the fix rather than failing obscurely at spawn time.
- **Networking is not enforced** inside the sandbox. `NetworkMode` intents
  still resolve at the manager level; in-workload enforcement is unimplemented,
  as it was before.

## Volumes and the network

A manifest names a mount point like `/data`, but the sandboxed process is a
native Windows program: there is no `/data`, and Windows has no `chroot` to
create one. So each granted volume is passed in the environment as
`TPT_VOLUME_<NAME>` holding its **host** path — the same convention
`tpt-runtime-linux` uses, so a workload reads its volumes the same way whichever
backend runs it. `TPT_NETWORK_MODE` names the granted mode.

Two things are deliberately not claimed:

- **Mounts are not confined.** The host path is reachable because the workload
  is an ordinary user-mode process. A `read-only` mount is a declaration, not
  an enforced restriction. `tpt inspect` reports `"enforced": false` on every
  mount for exactly this reason.
- **Network intent is advisory.** Per-workload networking on Windows needs the
  Host Network Service or per-app firewall rules, which this provider does not
  manage. `inspect` reports `"enforced": false` with a note.

What *is* enforced is consistency: a volume that does not exist, or a
read-write mount whose `filesystem.write` grant is missing, is a hard error at
start — not a silent downgrade. Likewise, ports granted to a
`network = "none"` workload, or an outbound mode without `network.outbound`,
are refused.

## Requirements

`CreateProcessAsUserW` requires `SeAssignPrimaryTokenPrivilege` and
`SeIncreaseQuotaPrivilege`, which a non-administrator process does not hold.
**The daemon must run elevated.**

The crate refuses to start an *unisolated* process as a fallback: if the
privileges cannot be enabled, `start` fails with an error saying so. Silently
dropping the restriction would leave the workload running with the daemon's
full privileges, which is the exact failure this crate exists to prevent.

## Usage

```rust
use std::sync::Arc;
use tpt_runtime_oci::OciBackend;

let backend = OciBackend::new(state_dir.join("images"))?
    .with_pull_policy(tpt_runtime_oci::PullPolicy::IfMissing)
    .with_provider(Arc::new(tpt_runtime_sandbox::WindowsSandbox::new()));
```

`OciBackend::prepare` resolves and pulls the image into a `Bundle`; `start`
hands that bundle to the provider, which returns a `WorkloadInstance`
implementing the usual `stats` / `stop` / `exit` / `describe` contract.

Without a provider, `start` still fails explicitly with `not_implemented` — it
never silently succeeds.

## Testing

```console
cargo test -p tpt-runtime-sandbox
```

The unit tests cover bundle resolution, environment construction and Windows
command-line quoting. `tests/isolation.rs` spawns real processes through the
whole path — a copy of the host `cmd.exe` placed in a throwaway rootfs — and
asserts on exit codes, captured stdout, stop semantics and environment
containment.

Those end-to-end tests need the privileges above. Run them from an elevated
shell; on an unelevated one they skip with an explanatory message rather than
reporting a false failure. No network access and no registry pulls are
involved.

## License

Licensed under either of [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.