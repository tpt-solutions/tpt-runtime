# Changelog

All notable changes to `tpt-runtime-storage` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- An Archon-backed `StorageManager` implementing the same surface with page
  cache and deduplication (SPEC §16).
- Enforce read-only mounts inside the volume layer rather than relying on
  backend enforcement and policy.
- Volume quota and usage reporting per volume.

### Added

- Corrupt `volume.json` metadata fails `StorageManager::open` loudly instead
  of dropping the volume silently (SPEC §45 storage failure, §48).

## [0.1.0]

Initial release: directory-backed logical volumes.

### Added

- **`StorageManager`** (`volume`): `open`, `reload`, `create`, `get`, `list`,
  `remove`, `mount_path` and `base_dir`, backed by a host directory.
- **`Volume`** (`volume`): a stable `VolumeId`, logical name, absolute backing
  path and creation time, with `supports_mount` reporting which access modes
  the volume can serve.
- **`VolumeInfo`** (`volume`): the listing projection of a volume, with the
  backing path as a display string.
- Explicit failure over silent reuse: creating a duplicate name returns
  `StorageFailure`, and missing volumes return `NotFound` from `get` and
  `remove`.
- Idempotent `reload` to re-scan the backing directory after out-of-band
  changes.
- Documented seam for `tpt-archon` integration: a different substrate
  implements the same `create`/`get`/`list`/`mount_path`/`remove` surface.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
