# Changelog

All notable changes to `tpt-runtime-core` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Re-export `ErrorKind` from the crate root once the API surface is frozen.
- `Timestamp` helpers for RFC 3339 parsing, to complement `time` formatting.

## [0.1.0]

Initial release: the platform-independent primitives every other
`tpt-runtime` crate builds on.

### Added

- **Identifiers** (`id`): opaque string newtypes `WorkloadId`, `VolumeId`,
  `NetworkId`, `DeviceId`, `CapabilityId`, `ServiceId` and `ResourceId`.
  Each exposes `PREFIX`, `generate()` (prefix-tagged UUIDv4), `from_raw`,
  `as_str`, `Display` and `AsRef<str>`, and serializes transparently over JSON.
- **Lifecycle state machine** (`state`): `WorkloadState` with `is_active`,
  `is_terminal` and a validating `transition` method, plus the
  `LIFECYCLE_TRANSITIONS` table. Permits the canonical progression, pause and
  resume, restart from `stopped`, failure from every active state, and treats
  `destroyed` as terminal.
- **Errors** (`error`): `RuntimeError` carrying an `ErrorKind` plus optional
  workload, backend and operation attribution, with `From` conversions for
  `std::io::Error` and `serde_json::Error`, and a `Result` alias.
- **`ErrorKind`** (`error`): twelve failure categories, all serde-compatible
  and round-trippable through `FromStr`/`Display` as snake_case.
- **Events** (`event`): `EventKind` covering the SPEC §28 event vocabulary with
  stable dotted names, and `RuntimeEvent` with builder-style attribution and
  flattened payload fields.
- **Timestamps** (`timestamp`): `Timestamp` wrapping `OffsetDateTime` with
  `now`, `as_offset` and `unix_seconds`.
- **Resource usage** (`usage`): `ResourceUsage` with `total_cpu`, peak
  `max_with` merging, nanosecond duration serialization and omission of
  zero-valued fields.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
