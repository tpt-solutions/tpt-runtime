# Changelog

All notable changes to `tpt-runtime-model` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- `Default` for `WorkloadSpec`, so partially-specified workloads can be built
  with `..Default::default()`.
- Re-export `ImageReference` from the crate root alongside `ExecutionSpec`.

## [0.1.0]

Initial release: the backend-independent workload model.

### Added

- **`WorkloadSpec`** (`workload`): the runtime's primary abstraction, bundling
  execution, resources, volumes, network, devices, requested capability names
  and free-form labels, with `new`, `backend` and `validate`. Validation covers
  name charset and length, volume mount completeness, and the rule that a
  network mode unable to accept inbound traffic may not expose ports.
- **`ExecutionSpec`** (`execution`): an enum with one populated variant per
  backend — `WindowsProcess`, `LinuxProcess`, `OciImage`, `WasmModule` — each
  carrying requirements rather than mechanisms, plus `backend` and `validate`.
- **`WasmModuleSpec`** and **`WasmClass`** (`execution`): module path and the
  `command` / `service` / `function` runtime classes (SPEC §14).
- **`ImageReference`** (`execution::oci_ref`): parsing of
  `name[:tag][@digest]` with registry hosts, distinguishing a registry port
  from a tag and requiring `sha256:` digests.
- **`BackendKind`** (`backend`): `Windows`, `Linux`, `Oci`, `Wasm`.
- **`ResourceSpec`** and **`Memory`** (`resources`): optional CPU, memory and
  timeout requests, with a byte count that parses `KiB`/`MiB`/`GiB` suffixes
  and rejects unknown units instead of silently zeroing.
- **`NetworkSpec`** and **`NetworkMode`** (`network`): `None` (the default, deny
  by default), `Outbound`, `Service` and `Host`, with `allows_outbound` and
  `allows_inbound` predicates and a named `expose` port map.
- **`VolumeMount`** and **`VolumeAccessMode`** (`volume`): logical volume
  binding to a mount point, resolved to host paths later per instance.
- **`DeviceRequest`** and **`DeviceAccessMode`** (`device`): logical device ids
  such as `gpu:0` with `Compute`, `Full` and `ReadOnly` access modes.
- Full `serde` support for every type, with snake_case representation so a
  manifest, an API request and a stored record share one shape.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
