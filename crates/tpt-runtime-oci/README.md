# tpt-runtime-oci

[![crate](https://img.shields.io/badge/crate-tpt--runtime--oci-orange)](https://crates.io/crates/tpt-runtime-oci)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--oci-blue)](https://docs.rs/tpt-runtime-oci)

OCI compatibility through Boxcar-compatible primitives (SPEC §13): image
reference resolution, a content-addressed image store, and bundle preparation
and validation.

> **Status.** References, the content store and bundle preparation are
> implemented and tested. `start` is the reserved seam: running a bundle in
> isolation awaits the `tpt-boxcar` isolation provider, and reports
> `NotImplemented` until then.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-oci = "0.1"
```

## The store layout

```text
<root>/
  images/
    refs/<repository-encoded>/<tag>     -> digest file
    blobs/sha256/<digest>               -> unpacked bundle directory
```

## Opening a store and registering bundles

```rust
use tpt_runtime_oci::{Bundle, OciBackend};

let dir = std::env::temp_dir().join(format!("tpt-oci-{}", std::process::id()));
let backend = OciBackend::new(&dir).unwrap();
let store = backend.store();

// Seed a bundle: the directory must contain a `rootfs`.
let bundle_dir = dir.join("seeded");
std::fs::create_dir_all(bundle_dir.join("rootfs")).unwrap();

let digest = "sha256:".to_owned() + &"a".repeat(64);
let blob = store.register_digest(&digest, &bundle_dir).unwrap();
assert!(blob.ends_with(&digest["sha256:".len()..]));

// Tagging pins a reference to a digest.
use tpt_runtime_model::execution::oci_ref::ImageReference;
let reference = ImageReference::parse("postgres:16").unwrap();
store.tag(&reference, &digest).unwrap();
```

`register_digest` refuses a directory with no `rootfs` rather than registering
something that cannot be run.

## Resolving a reference

```rust
use tpt_runtime_model::execution::oci_ref::ImageReference;
use tpt_runtime_oci::{Bundle, OciBackend};

let dir = std::env::temp_dir().join(format!("tpt-oci-resolve-{}", std::process::id()));
let backend = OciBackend::new(&dir).unwrap();
let store = backend.store();

let bundle_dir = dir.join("seeded");
std::fs::create_dir_all(bundle_dir.join("rootfs")).unwrap();
let digest = "sha256:".to_owned() + &"b".repeat(64);
store.register_digest(&digest, &bundle_dir).unwrap();

let reference = ImageReference::parse("postgres:16").unwrap();
store.tag(&reference, &digest).unwrap();

let bundle = store.resolve(&reference).unwrap();
assert!(bundle.config_path().exists() || bundle.rootfs_path().exists());
```

An unknown image is reported as `NotFound` and names the Boxcar dependency:
network pulls are deliberately absent from this crate.

## Bundle validation
```rust
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::OciImageSpec;
use tpt_runtime_model::execution::oci_ref::ImageReference;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_oci::{Bundle, OciBackend};

let dir = std::env::temp_dir().join(format!("tpt-oci-validate-{}", std::process::id()));
let backend = OciBackend::new(&dir).unwrap();

// Seed a bundle so the image reference resolves locally.
let bundle_dir = dir.join("seeded");
std::fs::create_dir_all(bundle_dir.join("rootfs")).unwrap();
let digest = "sha256:".to_owned() + &"c".repeat(64);
backend.store().register_digest(&digest, &bundle_dir).unwrap();
let reference = ImageReference::parse("postgres:16").unwrap();
backend.store().tag(&reference, &digest).unwrap();

// `resolve` returns a bundle with no entry point, so `prepare_bundle` falls
// back to `/bin/sh` -- an absolute path that `validate` rejects as escaping
// the rootfs. In other words a bare store cannot be started as-is: a real
// image supplies a relative entry point through its config.
let spec = WorkloadSpec::new(
    "api",
    ExecutionSpec::OciImage(OciImageSpec {
        image: "postgres:16".to_owned(),
        ..Default::default()
    }),
);

let err = backend.prepare_bundle(&spec).unwrap_err();
assert_eq!(err.kind, ErrorKind::InvalidConfiguration);
assert!(err.message.contains("escapes the rootfs"));

// The validation rules themselves, exercised on a Bundle directly.
// `register_digest` adopts the directory, so read the blob path back to
// get the bundle's real location.
let mut bundle = Bundle {
    id: "demo".to_owned(),
    path: backend.store().root().join("blobs").join("sha256").join(&digest["sha256:".len()..]),
    args: vec!["bin/sh".to_owned()],
    env: Default::default(),
    working_dir: None,
};
assert!(bundle.rootfs_path().is_dir());
assert!(bundle.validate().is_ok());

// An entry point that traverses out of the rootfs is rejected.
bundle.args = vec!["../../etc/passwd".to_owned()];
assert!(bundle.validate().is_err());

// An absolute entry point is rejected too.
bundle.args = vec!["/bin/sh".to_owned()];
assert!(bundle.validate().is_err());

// No entry point at all is rejected.
bundle.args = Vec::new();
assert!(bundle.validate().is_err());

// A bundle with no rootfs is rejected.
bundle.args = vec!["bin/sh".to_owned()];
bundle.path = dir.join("nowhere");
assert!(bundle.validate().is_err());

// An image that is not in the local store fails explicitly, naming Boxcar.
let missing = WorkloadSpec::new(
    "missing",
    ExecutionSpec::OciImage(OciImageSpec {
        image: "ghcr.io/nope/app:1.0".to_owned(),
        ..Default::default()
    }),
);
let err = backend.prepare_bundle(&missing).unwrap_err();
assert_eq!(err.kind, ErrorKind::NotFound);
assert!(err.message.contains("tpt-boxcar"));

```

## Starting a workload

`OciBackend::start` is the documented integration point for `tpt-boxcar`. The
backend prepares and validates a bundle; the Boxcar primitive supplies image
pull, layer unpack and the isolation boundary. Until that exists, `start`
returns `NotImplemented` rather than pretending to run something.

## Testing

```console
cargo test -p tpt-runtime-oci
```

Covers the store layout, digest registration and validation, tag pinning,
reference resolution, bundle preparation, and the explicit `NotImplemented`
path for `start`.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
