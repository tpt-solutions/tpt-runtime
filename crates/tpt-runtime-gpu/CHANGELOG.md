# Changelog

All notable changes to `tpt-runtime-gpu` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Per-GPU telemetry: utilization, temperature and memory occupancy, sampled
  alongside discovery.
- Support for AMD and Intel GPUs alongside NVIDIA.
- Enumerating devices by UUID rather than index, for stable identity across
  reboots.
- Multi-GPU topologies and interconnect (NVLink) reporting.

### Added

- GPU telemetry (SPEC §20): `sample_gpus` (utilization, memory, temperature,
  power via `nvidia-smi`), `GpuSample`, and `GpuTelemetry` — a latest-value
  cache refreshed by a background sampler, tolerant of failed polls.

## [0.1.0]

Initial release: NVIDIA discovery via `nvidia-smi`.

### Added

- **`discover_gpus`** (`discover`): shells out to `nvidia-smi` with a CSV query
  for index, name, total memory, driver version and UUID.
- **`GpuInfo`** (`discover`): index, name, total memory in MiB, driver version
  and optional UUID, with `device_id()` yielding the logical `gpu:<index>` id
  and `as_device()` producing a `DeviceInfo` registry entry.
- **Discovery versus failure** (SPEC §48): an absent `nvidia-smi` or a driver
  reporting no devices yields `Ok(vec![])`, while a present-but-broken stack
  returns `DeviceUnavailable` carrying the driver's own message.
- Filtering of the driver's `[N/A]` placeholder UUID.
- GPU grants are capability decisions: a workload must request the
  `device:gpu:0` capability, and nothing is attached ambiently.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
