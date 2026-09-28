# Changelog

All notable changes to `tpt-runtime-device` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Hot-plug: re-enumerate devices and emit `device.detached` when hardware
  disappears under a running workload.
- Shared device claims with explicit access-mode compatibility rules.
- Per-device telemetry (utilization, throughput) surfaced through the
  observability layer.

## [0.1.0]

Initial release: the logical device registry and claim tracking.

### Added

- **`DeviceRegistry`** (`registry`): `new`, `register`, `list`, `attach`,
  `detach`, `release_workload` and `claims_of`.
- **`DeviceInfo`** (`registry`): a logical `DeviceId`, a `DeviceClass`, a
  human-readable description and an availability flag.
- **`DeviceClass`** (`registry`): with `of_id` inferring a class from a logical
  id such as `gpu:0` or `serial:com3`, so host enumerators need not repeat the
  mapping.
- **Devices as capabilities**: attachment is an explicit claim requiring both a
  device request and a matching `device:<id>` capability grant.
- **Access-mode-driven exclusivity**: `Compute` and `ReadOnly` claims may be
  held by several workloads, while `Full` is exclusive and is refused with
  `DeviceUnavailable` while another workload holds a claim.
- **Explicit failure**: attaching an unknown or unavailable device returns
  `DeviceUnavailable`, and detaching a device with no claims returns
  `NotFound`.
- **`release_workload`**: releases every claim a workload held, so a destroyed
  or crashed workload cannot leak a device.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
