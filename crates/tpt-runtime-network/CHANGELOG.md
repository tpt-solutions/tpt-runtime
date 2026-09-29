# Changelog

All notable changes to `tpt-runtime-network` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- A `tpt-boxcar` service-mesh backend taking over `Service` name resolution
  while the manifest intent stays identical.
- Per-workload bandwidth accounting and inbound connection tracking.
- IPv6 and host-name based exposure in addition to host ports.

### Fixed

- Dynamic port allocation fails explicitly with `NetworkFailure` when no
  ephemeral port can be picked, instead of handing out port 0.

## [0.1.0]

Initial release: turning network intent into concrete port allocations.

### Added

- **`NetworkManager`** (`manager`): `new`, an async `assign` resolving a
  workload's `NetworkSpec` into a concrete assignment, and `release` freeing
  held ports.
- **`NetworkAssignment`** and **`PortAllocation`** (`manager`): the resolved
  network id, mode and per-port host allocations, with `static_port`
  distinguishing a fixed manifest port from a dynamically chosen one.
- **Dynamic port selection**: a requested port of `0` is bound on an ephemeral
  socket and the OS-assigned port reported, retried on collision.
- **Explicit failure over silent degradation** (SPEC §48): exposing a port
  under a mode that cannot accept inbound traffic returns `NetworkFailure`, and
  a duplicate static port is refused rather than stealing the port.
- **Port release**: stopping a workload frees its ports, making fixed ports
  available to the next workload.
- Deterministic ordering: allocated ports are sorted by logical name so
  assignments serialize stably.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
