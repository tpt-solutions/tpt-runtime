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
