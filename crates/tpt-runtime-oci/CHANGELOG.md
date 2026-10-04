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

### Fixed

- `ImageStore::resolve` validates the digest read from a tag file before it
  touches a path, so a poisoned ref cannot point outside the blob store.
  Stored refs accept both the bare hex form written by `tag` and the
  `sha256:`-prefixed form.
- `Bundle::validate` rejects Windows-style entry points (backslashes,
  drive-letter colons) in addition to absolute paths and `..` traversal.

### Added

- Malicious image test suite (SPEC §46): poisoned tags, malformed digests,
  rootfs-less blobs and hostile bundle entry points.

### Added

- **Registry pull** (SPEC §13): Docker Registry HTTP API v2 client —
  anonymous Bearer token flow, manifest and index resolution (picks
  `linux/amd64`, falls back to any linux), streaming blob downloads with
  sha256 verification and a configurable size cap (decompression-bomb
  guard). Digest-pinned references verify served bytes before use.
- **Layer unpacking**: tar and tar+gzip layers extracted in manifest order
  with overlay whiteout semantics (`.wh.<name>` deletes, `.wh..wh..opq`
  makes a directory opaque); hostile layers are rejected loudly — `..`
  traversal, absolute escapes and out-of-rootfs symlink targets never
  touch the host; per-file extraction cap; unix mode bits preserved.
- **Content store rework**: raw blobs (manifests, configs, layers) live
  content-addressed under `blobs/sha256/`, unpacked bundles under
  `bundles/<digest>/` with the image's run defaults (`image.json`) and a
  runtime-spec `config.json`. Cached layers skip re-download; a tagged and
  unpacked image short-circuits a pull entirely.
- **Pull pipeline + policy**: `OciBackend` gains `PullPolicy`
  (`Never` by default for the library; `IfMissing` in the daemon, disable
  with `TPT_RUNTIME_OCI_PULL=0`), `with_max_blob_bytes`, and loopback
  registries served over plain HTTP. `resolve` fills bundle argv/env/cwd
  from the image config (real PascalCase keys).
- Fake-registry integration suite (SPEC §45): end-to-end pull with token
  auth, whiteouts and cache reuse; corrupt-blob digest rejection; index
  platform selection; backend pull-policy path.

### Fixed

- `resolve` now derives `args`/`env`/`working_dir` from the stored image
  defaults instead of returning an empty bundle; user args still append.

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
