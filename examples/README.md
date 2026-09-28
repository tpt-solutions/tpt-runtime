# tpt-runtime examples

Runnable manifests for the `tpt` CLI. Every file here is a valid
`tpt.runtime/v1` document that the runtime can parse and validate.

## Prerequisites

Start the daemon first:

```console
tpt daemon start
```

## The examples

| File | Shows |
| --- | --- |
| `windows-echo.toml` | the smallest useful manifest: one process, no network |
| `windows-service.toml` | service mode, a fixed port, a volume, limits, labels |
| `windows-limited.toml` | over-requests that policy denies or clamps |
| `windows-gpu.toml` | a device request plus the matching capability |
| `windows-secret.toml` | a secret held behind a capability, not an env var |
| `wasm-hello.toml` | a WASM module bounded by fuel and a timeout |

## Running them

```console
tpt run --manifest examples/windows-echo.toml
tpt run --manifest examples/windows-service.toml
tpt run --manifest examples/windows-limited.toml
tpt run --manifest examples/windows-gpu.toml
tpt run --manifest examples/wasm-hello.toml
```

`windows-gpu.toml` needs a host with an NVIDIA GPU; `windows-secret.toml`
needs a secret to exist first (`tpt secret set github-token`).

You can also skip manifests entirely and run directly:

```console
tpt run --windows cmd.exe -- /C echo hello
tpt run --wasm examples/hello.wasm
```

## Adapting an example

Each manifest is a good starting point for a real workload. The usual edits
are the executable and its arguments, the resource requests, the volume mounts,
and the capability list. Remember that capabilities are the only authority a
workload has: if something fails with a capability error, grant that capability
explicitly rather than looking for an ambient-access switch.
