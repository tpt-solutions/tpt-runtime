# Changelog

All notable changes to `tpt-runtime-windows` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Console/pty support so interactive workloads can be attached to.
- Graceful shutdown for console applications that handle `CTRL_BREAK_EVENT`.
- Per-process (not only per-job) CPU attribution in `ResourceUsage`.
- Windows sandboxing beyond Job Objects, via `tpt-boxcar` primitives.

### Added

- Job-object tests (SPEC §45): kill-on-close reaps the workload when the
  last handle drops (runtime restart safety), and a per-process commit
  ceiling starves an allocating workload while an unlimited control keeps
  running (OS-level resource exhaustion).

## [0.1.0]

Initial release: native Windows process execution.

### Added

- **`WindowsProcessBackend`** (`backend`): an `ExecutionBackend` implementing
  `kind`, an idempotent `prepare` and `start`, for the `windows` backend kind.
- **Job Object isolation** (`backend::job`): every workload process runs in a
  dedicated job, so termination is job-wide and child processes cannot outlive
  the workload.
- **Kill-on-close**: `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ties workload lifetime
  to the daemon's, so a crashed daemon cannot leak workload processes.
- **Memory limits** enforced by the job when the manifest requests one.
- **Sanitized environment**: the child inherits an allowlist rather than the
  daemon's environment, with manifest values layered on top.
- **stdout/stderr capture** into the shared `LogCapture` buffers, so `tpt logs`
  behaves as it does for every other backend.
- **Resource accounting**: job counters feed `ResourceUsage` (user and kernel
  CPU, peak commit charge, bytes read and written).
- **Cross-platform compilation**: the crate builds on any host; on non-Windows
  systems operations return `BackendUnavailable` instead of panicking.
- Documented that both `StopMode::Graceful` and `StopMode::Kill` are treated
  as job-wide termination, since native Windows processes have no portable
  graceful-shutdown contract.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
