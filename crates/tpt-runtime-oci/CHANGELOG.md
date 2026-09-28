# Changelog

All notable changes to `tpt-runtime-oci` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- `start` backed by the `tpt-boxcar` isolation provider: image pull, layer
  unpack and the isolation boundary.
- Registry resolution, replacing the current resolve-only-against-local-blobs
  behaviour.
- Bundle configuration assembly from the workload model (env, mounts,
  capabilities) rather than only validating what was seeded.

## [0.1.0]

Initial release: OCI references, content store and bundle preparation.

### Added

- **`OciBackend`** (`backend`): `new`, `store` and `prepare_bundle`, the
  reserved seam for Boxcar-backed execution.
- **`ImageStore`** (`store`): a content-addressed store with
  `open`/`root`/`register_digest`/`tag`/`resolve`, laid out as
  `images/refs/<repo>/<tag>` pointing at `images/blobs/sha256/<digest>`.
- **Defensive registration**: a bundle directory without a `rootfs` is
  rejected rather than registered, and digests are validated before use.
- **Tag pinning** and reference resolution against locally present blobs, with
  a clear `NotFound` naming the Boxcar dependency for anything not present.
- **`Bundle`** (`bundle`): `config_path`, `rootfs_path` and `validate`.
- **Reference parsing** delegated to
  `tpt_runtime_model::execution::oci_ref::ImageReference`, including registry
  hosts and `sha256:` digest validation.
- **Explicit pending state**: `start` reports `NotImplemented` while the
  Boxcar isolation provider is unavailable, rather than silently succeeding.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
