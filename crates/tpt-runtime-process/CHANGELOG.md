# Changelog

All notable changes to `tpt-runtime-process` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- A `prepare`/`start` split that supports asynchronous preparation for
  slow-to-materialize backends such as OCI image pulls.
- Streaming log tailing that does not require draining the whole buffer.

## [0.1.0]

Initial release: the backend extension point.

### Added

- **`ExecutionBackend`** (`backend`): the trait every execution backend
  implements, with `kind`, an idempotent non-starting `prepare`, and `start`
  returning a live `Box<dyn WorkloadInstance>`. Methods are synchronous by
  design; backends do their own threading and the manager calls them off the
  async runtime's core threads.
- **`WorkloadInstance`** (`backend`): `stats` for usage snapshots, `stop` with
  a `StopMode`, `exit` returning a `oneshot::Receiver<ExitStatus>` that must
  complete even on kill or crash, and an optional `describe` used by
  `tpt inspect`.
- **`StartContext`** (`backend`): everything a backend needs to prepare and
  start one workload, carrying the assigned id, resolved mounts, log directory,
  the granted `CapabilitySet`, the network mode and any exposed host ports.
- **`ResolvedMount`** (`backend`): a volume name bound to its per-instance host
  path and access mode, realizing one volume as many views (SPEC §15).
- **`StopMode`** (`backend`): `Graceful` and `Kill`, documented so backends
  without graceful shutdown semantics state how they treat both.
- **`ExitStatus`** (`backend`): exit code, killed and failed flags, with
  `success` and `terminated` constructors.
- **`LogCapture`** and **`LogBuffer`** (`logs`): bounded in-memory tailing plus
  file-backed capture, with `create`, `spawn_reader`, `push`, `tail`, `len` and
  `is_empty`.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
