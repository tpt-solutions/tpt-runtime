# Changelog

All notable changes to `tpt-runtime-daemon` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Graceful restart with workload reconciliation, adopting workloads that
  outlived the previous daemon process.
- Configurable admission policy from a daemon configuration file, rather than
  the current built-in default.
- Windows service registration for unattended startup.
- Signal-driven shutdown on SIGTERM/SIGINT equivalents.

## [0.1.0]

Initial release: the Windows host daemon.

### Added

- **`run`** (`lib`): the entire daemon as one async entry point — prepares
  directories, builds every subsystem, discovers devices, constructs the
  workload manager with a policy engine derived from host capacity, registers
  the backends, and serves the local API until shutdown.
- **`tpt-runtime-daemon` binary** (`main`): a thin wrapper around `run`,
  honouring `--state-dir` and `--pipe` plus the `TPT_RUNTIME_DIR` and
  `TPT_RUNTIME_PIPE` environment variables.
- **Full subsystem assembly**: the event hub with its JSONL sink, the metrics
  registry, storage, network, devices, secrets and the policy engine.
- **GPU discovery at startup**, registering each `nvidia-smi` result as a
  `gpu:<index>` device in the device registry.
- **Backend registration** for Windows, WASM, OCI and Linux, in one place.
- **Process lifetime tied to workloads** (SPEC §12, SPEC §45): native
  workloads run in Job Objects with kill-on-close, so a daemon crash cannot
  leave orphaned workload processes behind.
- **Durable state**: event history and logs persist to disk, so a restarted
  daemon can still report what happened before it stopped.
- Documented state directory layout: `logs/`, `volumes/`, `events.jsonl` and
  `secrets.json`.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
