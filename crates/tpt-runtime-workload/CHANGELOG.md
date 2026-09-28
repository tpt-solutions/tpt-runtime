# Changelog

All notable changes to `tpt-runtime-workload` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- `resume`, complementing `pause`, once a backend supports suspension.
- Reconciliation on daemon restart: adopt workloads that outlived the daemon
  and mark those that did not.
- Priority-aware scheduling, using `tpt-runtime-policy::ResourcePolicy::Priority`
  once capacity is reclaimable.
- Workload dependencies, so a workload can be gated on another's health.

## [0.1.0]

Initial release: one lifecycle across every execution backend.

### Added

- **`WorkloadManager`** (`manager`): `new` taking the coordinated subsystems,
  `register_backend`, `create`, `start`, `wait`, `stop_and_wait`, `restart`,
  `pause`, `destroy`, `list`, `inspect`, `usage` and `logs`, plus
  `set_default_policy`.
- **State machine enforcement**: every transition is validated against
  `tpt_runtime_core::WorkloadState::transition`, so an illegal move fails with
  `InvalidTransition` instead of corrupting a record.
- **Pre-start resolution**: volumes, networks and devices are resolved and
  claimed before a backend is asked to start anything.
- **Capability recording**: granted capability names are exposed in
  `WorkloadInfo`, so a workload's authority is inspectable.
- **Normalized observability**: every backend's events and usage flow through
  `tpt-runtime-observe`, so telemetry does not vary by backend.
- **`WorkloadInfo` and `MountInfo`** (`info`): serializable views covering
  state, backend, capabilities, network mode, exposed ports, mounts, usage and
  exit status, backing `tpt list` and `tpt inspect`.
- **`LogsQuery`** (`manager`): selecting stdout or stderr, with a tail bound.
- **Subsystem accessors** for the event hub, metrics registry, storage,
  network, device and secret stores.
- **Explicit failure**: a workload that cannot start fails with a reason and
  its record moves to `failed` rather than disappearing.
- Documented that `pause`/`resume` report `not_implemented` until a backend
  supports suspension.
- End-to-end tests against the real Windows backend, including the crash
  watcher that emits `workload.failed`.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
