# tpt-runtime-device

[![crate](https://img.shields.io/badge/crate-tpt--runtime--device-orange)](https://crates.io/crates/tpt-runtime-device)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--device-blue)](https://docs.rs/tpt-runtime-device)

The device model (SPEC §19): **devices are capabilities.**

A workload asks for `gpu:0` or `serial:com3` by logical id, and the runtime
decides how that access is realized on this host. The registry tracks which
logical devices exist and which workloads have claimed them.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-device = "0.1"
```

## Registering devices

```rust
use tpt_runtime_device::{DeviceClass, DeviceInfo, DeviceRegistry};

let mut registry = DeviceRegistry::new();

registry.register(DeviceInfo {
    id: "gpu:0".into(),
    class: DeviceClass::Gpu,
    description: "Test GPU (8192 MiB)".to_owned(),
    available: true,
});

let devices = registry.list();
assert_eq!(devices.len(), 1);
assert_eq!(devices[0].id.as_str(), "gpu:0");
```

`DeviceClass::of_id` infers a class from an id, so `gpu:0` classifies as
`Gpu` without the caller having to say so.

## Attaching a device to a workload

Attachment is an explicit claim. A device that is already claimed is refused
rather than silently shared.

```rust
use tpt_runtime_model::device::DeviceAccessMode;
use tpt_runtime_device::{DeviceClass, DeviceInfo, DeviceRegistry};

let mut registry = DeviceRegistry::new();
registry.register(DeviceInfo {
    id: "gpu:0".into(),
    class: DeviceClass::Gpu,
    description: "Test GPU".to_owned(),
    available: true,
});

let attached = registry.attach("wl-trainer", "gpu:0", DeviceAccessMode::Compute).unwrap();
assert_eq!(attached.id.as_str(), "gpu:0");

// `Compute` is shareable, so a second workload may also attach.
registry.attach("wl-tuner", "gpu:0", DeviceAccessMode::Compute).unwrap();
assert_eq!(registry.claims_of("gpu:0").len(), 2);

// `Full` is exclusive: it is refused while another workload holds a claim.
let err = registry.attach("wl-exclusive", "gpu:0", DeviceAccessMode::Full).unwrap_err();
assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::DeviceUnavailable);
assert!(err.message.contains("exclusive mode"));

// An unknown device is an explicit failure, not a silent no-op.
let err = registry.attach("wl-other", "gpu:9", DeviceAccessMode::Compute).unwrap_err();
assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::DeviceUnavailable);
```

## Detaching and releasing

```rust
use tpt_runtime_device::DeviceRegistry;
use tpt_runtime_model::device::DeviceAccessMode;

let mut registry = DeviceRegistry::new();
registry.register(tpt_runtime_device::DeviceInfo {
    id: "serial:com3".into(),
    class: tpt_runtime_device::DeviceClass::of_id("serial:com3"),
    description: "COM3".to_owned(),
    available: true,
});

registry.attach("wl-serial", "serial:com3", DeviceAccessMode::Full).unwrap();
assert_eq!(registry.claims_of("serial:com3"), vec!["wl-serial".to_owned()]);

// Detach one workload, or release everything a workload held at teardown.
registry.detach("wl-serial", "serial:com3").unwrap();
assert!(registry.claims_of("serial:com3").is_empty());

let err = registry.detach("wl-serial", "serial:com3").unwrap_err();
assert_eq!(err.kind, tpt_runtime_core::error::ErrorKind::NotFound);
```

`release_workload` is what the manager calls when a workload is destroyed, so
a crashed workload never leaks a device claim.

```rust
use tpt_runtime_device::{DeviceClass, DeviceInfo, DeviceRegistry};
use tpt_runtime_model::device::DeviceAccessMode;

let mut registry = DeviceRegistry::new();
for id in ["gpu:0", "gpu:1"] {
    registry.register(DeviceInfo {
        id: id.into(),
        class: DeviceClass::Gpu,
        description: id.to_owned(),
        available: true,
    });
}

registry.attach("wl-a", "gpu:0", DeviceAccessMode::Compute).unwrap();
registry.attach("wl-a", "gpu:1", DeviceAccessMode::Compute).unwrap();

registry.release_workload("wl-a");

assert!(registry.claims_of("gpu:0").is_empty());
assert!(registry.claims_of("gpu:1").is_empty());
```

## Requesting a device from a workload

```toml
[[devices]]
id = "gpu:0"
mode = "compute"   # or "full", or "read-only"

[[capabilities]]
name = "device:gpu:0"
```

Both the device request and the matching capability are required: the request
says what is needed, the capability is the authorization.

## Device classes

`DeviceClass::of_id` recognizes the `gpu:`, `serial:`, `usb:` and related
prefixes so a host enumerator can classify a device without repeating the
mapping.

## Testing

```console
cargo test -p tpt-runtime-device
```

Covers class inference, registration, exclusive attachment, unknown devices,
detachment and workload release.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
