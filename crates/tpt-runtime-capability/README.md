# tpt-runtime-capability

[![crate](https://img.shields.io/badge/crate-tpt--runtime--capability-orange)](https://crates.io/crates/tpt-runtime-capability)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--capability-blue)](https://docs.rs/tpt-runtime-capability)

The capability model (SPEC §5.2, §23): workloads receive only the access that is
explicitly granted. **Ambient host authority does not exist.**

Capabilities are declared by dotted names in manifests, parsed into typed
`Capability` values that backends can enforce, and every check outcome is
attributable. Grants are revocable at any time.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-capability = "0.1"
```

## The capability names

| Manifest name | Variant | Grants |
| --- | --- | --- |
| `filesystem.read` | `FilesystemRead` | read workload-visible volumes |
| `filesystem.write` | `FilesystemWrite` | write to read-write mounted volumes |
| `network.outbound` | `NetworkOutbound` | outbound connections |
| `network.inbound` | `NetworkInbound` | bind inbound listeners |
| `network.host` | `NetworkHost` | full host network access |
| `device:gpu:0` | `Device { id }` | access to a specific device |
| `secret:github-token` | `Secret { name }` | access to a named secret |
| `ipc:service:database` | `IpcService { service }` | another workload's endpoint |
| anything else | `Other { name }` | recognized but unenforced, kept for audit |

Names round-trip exactly, so a capability can be serialized for an audit log
and re-parsed without drift.

```rust
use tpt_runtime_capability::Capability;

for name in [
    "filesystem.read",
    "filesystem.write",
    "network.outbound",
    "network.inbound",
    "network.host",
    "device:gpu:0",
    "secret:token",
    "ipc:service:db",
] {
    assert_eq!(Capability::parse(name).name(), name);
}

// Unknown names are preserved, not dropped.
let other = Capability::parse("compiler.execute");
assert_eq!(other, Capability::Other { name: "compiler.execute".to_owned() });
```

## Deny by default

An empty set grants nothing. This is the single most important property of the
crate: a workload that asks for no capabilities can do no privileged work.

```rust
use tpt_runtime_capability::{Capability, CapabilitySet};

let set = CapabilitySet::empty();
assert!(set.is_empty());
assert!(!set.is_granted(&Capability::NetworkOutbound));
assert!(set.require(&Capability::FilesystemWrite).is_err());
```

`require` returns a `CapabilityDenied` error naming the missing capability,
which is what makes denials explainable in an audit trail.

```rust
use tpt_runtime_capability::{Capability, CapabilitySet};
use tpt_runtime_core::error::ErrorKind;

let set = CapabilitySet::empty();
let err = set.require(&Capability::parse("secret:github-token")).unwrap_err();

assert_eq!(err.kind, ErrorKind::CapabilityDenied);
assert!(err.message.contains("secret:github-token"));
```

## Granting, checking and revoking

```rust
use tpt_runtime_capability::{Capability, CapabilitySet};

let mut set = CapabilitySet::from_names(["network.outbound", "filesystem.read"]);
assert_eq!(set.len(), 2);

assert!(set.require(&Capability::NetworkOutbound).is_ok());

// Revocation is immediate and reports whether anything was removed.
assert!(set.revoke(&Capability::NetworkOutbound));
assert!(set.require(&Capability::NetworkOutbound).is_err());
assert!(!set.revoke(&Capability::NetworkOutbound)); // already gone

set.grant(Capability::FilesystemWrite);
assert!(set.is_granted(&Capability::FilesystemWrite));
```

Because backends consult the set before doing privileged work, a revoke
followed by a check denies the operation — that is the whole revocation model.

## Event-ready check results

`check` returns a `CapabilityCheck` rather than a bare `bool`, so callers can
emit `capability.granted` or `capability.denied` events (SPEC §28) without
reconstructing context.

```rust
use tpt_runtime_capability::{Capability, CapabilitySet};

let set = CapabilitySet::from_names(["network.outbound"]);

let allowed = set.check(Capability::NetworkOutbound);
assert!(allowed.granted);
assert_eq!(allowed.capability.name(), "network.outbound");

let denied = set.check(Capability::parse("secret:api-key"));
assert!(!denied.granted);
```

## Serialization for audit

`CapabilitySet` serializes as the array of granted names, which is a convenient
shape for API responses and audit records.

```rust
use tpt_runtime_capability::CapabilitySet;

let set = CapabilitySet::from_names(["network.outbound"]);
let value: serde_json::Value = serde_json::to_value(&set).unwrap();

assert!(value.is_array());
assert_eq!(value[0], "network.outbound");
```

## Enforcing a capability in a backend

The intended integration point. A backend asks before privileged work and turns
a denial into a structured event:

```rust
use tpt_runtime_capability::{Capability, CapabilityCheck, CapabilitySet};
use tpt_runtime_core::error::ErrorKind;
use tpt_runtime_core::{EventKind, RuntimeEvent, RuntimeError};

fn open_socket(caps: &CapabilitySet) -> tpt_runtime_core::Result<()> {
    let check: CapabilityCheck = caps.check(Capability::NetworkOutbound);

    let event = RuntimeEvent::now(if check.granted {
        EventKind::CapabilityGranted
    } else {
        EventKind::CapabilityDenied
    })
    .with_workload("wl-demo")
    .with_field("capability", check.capability.name());

    if !check.granted {
        return Err(RuntimeError::new(
            ErrorKind::CapabilityDenied,
            "network.outbound is required to open outbound sockets",
        ));
    }
    let _ = event; // in the runtime, emit on the EventHub
    Ok(())
}

let caps = CapabilitySet::from_names(["network.outbound"]);
assert!(open_socket(&caps).is_ok());
assert!(open_socket(&CapabilitySet::empty()).is_err());
```

## Testing

```console
cargo test -p tpt-runtime-capability
```

Tests cover name parsing and round-trips, unknown-name fallback, the
empty-set deny-everything invariant, grant and revoke behaviour, denial
messages, and set serialization.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.
