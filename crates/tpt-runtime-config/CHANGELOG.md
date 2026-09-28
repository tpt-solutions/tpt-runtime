# Changelog

All notable changes to `tpt-runtime-config` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- `tpt.runtime/v2` support: richer environment profiles and toolchain
  selection, per SPEC §32.
- Validation of volume names against the logical-name rules, so a bad name is
  caught at parse time rather than at mount time.

## [0.1.0]

Initial release: declarative configuration for workloads and the daemon.

### Added

- **`Manifest`** (`manifest`): the `tpt.runtime/v1` document, with `parse`,
  `from_path`, `validate_version` and `into_workload_spec`.
- **Strict parsing**: every table uses `deny_unknown_fields`, so misspelled
  keys and unknown tables are rejected instead of silently ignored.
- **Version pinning**: the `api` key is validated on every parse; a mismatch
  reports both the requested and the implemented version. The key defaults to
  the current version when omitted.
- **Tables** (`manifest`): `WorkloadTable`, `ExecutionTable`, `VolumeTable`,
  `DeviceTable` and `CapabilityTable`, mirroring the manifest layout while
  staying backend-neutral.
- **Conversion to the model**: `into_workload_spec` maps a manifest onto a
  validated `WorkloadSpec`, including volume and device access-mode parsing
  with sensible defaults (read-write, compute).
- **`MANIFEST_API_VERSION`**: the schema version this crate implements.
- **`DaemonConfig`** (`daemon`): state directory, API pipe name and client
  limit, with `from_env` honouring `TPT_RUNTIME_DIR` and `TPT_RUNTIME_PIPE`,
  derived path helpers (`logs_dir`, `volumes_dir`, `events_file`,
  `secrets_file`) and idempotent `prepare_dirs`.
- **`DEFAULT_PIPE_NAME`**: `\\.\pipe\tpt-runtime-api`.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
