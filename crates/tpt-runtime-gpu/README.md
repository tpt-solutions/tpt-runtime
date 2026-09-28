# tpt-runtime-gpu

[![crate](https://img.shields.io/badge/crate-tpt--runtime--gpu-orange)](https://crates.io/crates/tpt-runtime-gpu)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--gpu-blue)](https://docs.rs/tpt-runtime-gpu)

GPU discovery and telemetry for the "Windows host + NVIDIA GPU" MVP target
(SPEC §20).

Discovered GPUs are registered into the device registry as `gpu:<index>`
devices. Granting one to a workload is a **capability decision, never ambient**.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-gpu = "0.1"
```

## Discovering GPUs

```rust
use tpt_runtime_gpu::discover_gpus;

let gpus = discover_gpus().unwrap();

for gpu in &gpus {
    println!(
        "{} ({} MiB, driver {}, uuid {})",
        gpu.name, gpu.memory_total_mib, gpu.driver_version,
        gpu.uuid.as_deref().unwrap_or("unknown")
    );
    assert_eq!(gpu.device_id().as_str(), format!("gpu:{}", gpu.index));
}
```

Discovery shells out to `nvidia-smi` with a CSV query.

## No NVIDIA stack is not a failure

`discover_gpus` distinguishes three outcomes deliberately (SPEC §48):

| Situation | Result |
| --- | --- |
| `nvidia-smi` absent | `Ok(vec![])` — the host simply has no NVIDIA GPUs |
| Driver reports no devices | `Ok(vec![])` |
| Driver present but broken | `Err(DeviceUnavailable)` with the driver's message |

That distinction lets the runtime say "this host has no GPU" and "this host's
GPU stack is broken" as different things, instead of collapsing both into a
silent empty list.

```rust
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_gpu::discover_gpus;

match discover_gpus() {
    Ok(gpus) => println!("{} gpu(s) discovered", gpus.len()),
    Err(err) => {
        // Only a broken stack errors; absence is reported as an empty list.
        assert_eq!(err.kind, ErrorKind::DeviceUnavailable);
        println!("nvidia-smi is present but failing: {err}");
    }
}
```

## Bridging into the device registry

`as_device` converts a discovered GPU into a registry entry, so discovery and
device attachment use one vocabulary.

```rust
use tpt_runtime_device::DeviceClass;
use tpt_runtime_gpu::GpuInfo;

let gpu = GpuInfo {
    index: 0,
    name: "Test GPU".to_owned(),
    memory_total_mib: 8192,
    driver_version: "550.00".to_owned(),
    uuid: Some("GPU-test".to_owned()),
};

let device = gpu.as_device();
assert_eq!(device.class, DeviceClass::Gpu);
assert!(device.available);
assert!(device.description.contains("8192"));
assert_eq!(device.id.as_str(), "gpu:0");
```

## Requesting a GPU from a workload

Requesting a GPU is a manifest entry plus a matching capability — the runtime
never attaches one implicitly.

```toml
[[devices]]
id = "gpu:0"
mode = "compute"

[[capabilities]]
name = "device:gpu:0"
```

## Testing

```console
cargo test -p tpt-runtime-gpu
```

`discovery_is_safe_on_any_host` must pass on machines without `nvidia-smi`, and
validates entry shape when the tool is present.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
