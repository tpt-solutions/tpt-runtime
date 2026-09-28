# tpt-runtime-storage

[![crate](https://img.shields.io/badge/crate-tpt--runtime--storage-orange)](https://crates.io/crates/tpt-runtime-storage)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--storage-blue)](https://docs.rs/tpt-runtime-storage)

Logical volumes (SPEC §15): named, inspectable storage units that workloads
mount as views. The MVP implementation is directory-backed on the host; an
Archon-backed volume (SPEC §16) would implement the same `Volume` model.

```console
tpt volume create project
tpt volume list
```

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-storage = "0.1"
```

## Creating and inspecting volumes

```rust
use tpt_runtime_storage::StorageManager;

// A fresh directory per run keeps the example repeatable.
let dir = std::env::temp_dir().join(format!("tpt-vol-{}", std::process::id()));
let mut storage = StorageManager::open(dir).unwrap();

let volume = storage.create("project").unwrap();
assert_eq!(volume.name, "project");

// Creating the same name twice is an explicit error, not a silent reuse.
let err = storage.create("project").unwrap_err();
assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::StorageFailure);

// List what exists, with mount paths and sizes.
let infos = storage.list();
assert!(infos.iter().any(|i| i.name == "project"));
for info in &infos {
    println!("{} -> {}", info.name, info.backing_path);
}

// Fetch one volume, or fail with NotFound.
let volume = storage.get("project").unwrap();
let missing = storage.get("nope").unwrap_err();
assert_eq!(missing.kind, tpt_runtime_core::error::ErrorKind::NotFound);
```

## Resolving a mount path

A backend asks for the host path backing a logical volume when it builds a
`ResolvedMount`. This is the "one volume, many views" seam: the same volume can
be mounted into several workloads with different access modes.

```rust
use tpt_runtime_storage::StorageManager;

let dir = std::env::temp_dir().join(format!("tpt-vol-mount-{}", std::process::id()));
let mut storage = StorageManager::open(dir).unwrap();
storage.create("assets").unwrap();

let path = storage.mount_path("assets").unwrap();
assert!(path.ends_with("assets"));
assert!(path.exists());
```

## Access modes

`Volume` records which access modes it supports. A volume that is read-only
cannot satisfy a read-write request, so a backend can fail the request early
rather than silently degrading it.

```rust
use tpt_runtime_model::volume::VolumeAccessMode;
use tpt_runtime_storage::StorageManager;

let dir = std::env::temp_dir().join(format!("tpt-vol-modes-{}", std::process::id()));
let mut storage = StorageManager::open(dir).unwrap();
let volume = storage.create("data").unwrap();

// A freshly created volume supports both modes.
assert!(volume.supports_mount(VolumeAccessMode::ReadOnly));
assert!(volume.supports_mount(VolumeAccessMode::ReadWrite));
```

## Reloading and removing

```rust
use tpt_runtime_storage::StorageManager;

let dir = std::env::temp_dir().join(format!("tpt-vol-reload-{}", std::process::id()));
let mut storage = StorageManager::open(&dir).unwrap();
storage.create("scratch").unwrap();

// Re-scan the backing directory after out-of-band changes.
storage.reload().unwrap();

// Remove a volume; removing one that does not exist is an explicit error.
storage.remove("scratch").unwrap();
assert!(storage.get("scratch").is_err());
let err = storage.remove("scratch").unwrap_err();
assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::NotFound);
```

## Swapping in a different substrate

`StorageManager` is the documented seam for `tpt-archon` integration. A
deduplicating, page-cached store implements the same
`create`/`get`/`list`/`mount_path`/`remove` surface, and nothing above it
changes.

## Testing

```console
cargo test -p tpt-runtime-storage
```

Covers create/list/get/remove, duplicate creation, missing volumes, access-mode
support and reload.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
