# Changelog

All notable changes to `tpt-runtime-linux` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- **Phase 2**: tighter TPT resource-management integration for Linux
  workloads.
- **Phase 3**: a TPT-managed Linux execution environment, replacing the
  dependence on an externally managed WSL distribution.
- Isolation inside the distro, in coordination with `tpt-boxcar`.

### Fixed

- `stop` now terminates the workload: the `wsl.exe` relay is kept reachable
  and killed, instead of the previous no-op that stalled `workloads.stop`.
- The host environment no longer leaks into `wsl.exe`, and `wsl.exe` now
  receives the minimal Windows variables it needs to initialize (a bare
  `env_clear` broke launches).

### Added

- Capability translation (SPEC §23): manifest env and granted volumes cross
  the WSL boundary through `WSLENV`; mounts arrive as `TPT_VOLUME_<NAME>`
  with `/p` path translation (`D:\data` → `/mnt/data`).
- `describe` reports the distro and relay pid; `stats` reports captured
  I/O through the relay.
- Real-distro tests (lifecycle, env/volume translation, relay stop) that
  adapt to installed distros and skip gracefully on hosts without WSL.

## [0.1.0]

Initial release: the Phase 1 layered strategy.

### Added

- **`LinuxBackend`** (`backend`): an `ExecutionBackend` for the `linux` backend
  kind, running workloads through `wsl.exe` and exposing them via the common
  workload model.
- **Manifest support**: `[execution] backend = "linux"` with `distro` and
  `command`, carried by `LinuxProcessSpec` in the workload model.
- **Explicit prerequisites**: a missing WSL installation or distribution
  surfaces as a clear error rather than an obscure failure.
- Documented scope: isolation inside the distribution is out of scope, and
  capability grants are recorded and audited by the runtime rather than
  enforced by this backend.
- Documented the compatibility-versus-implementation distinction (SPEC §11):
  workloads need a Linux-compatible environment, not a TPT-controlled kernel.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
