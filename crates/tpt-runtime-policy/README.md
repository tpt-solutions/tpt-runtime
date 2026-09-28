# tpt-runtime-policy

[![crate](https://img.shields.io/badge/crate-tpt--runtime--policy-orange)](https://crates.io/crates/tpt-runtime-policy)
[![docs](https://img.shields.io/badge/docs.rs-tpt--runtime--policy-blue)](https://docs.rs/tpt-runtime-policy)

Declarative resource policies and admission decisions: which resource requests
are granted, clamped or denied (SPEC §25).

The engine is intentionally **pure**. It takes host capacity plus a request and
returns a decision. Applying that decision — actually capping a Job Object or
a wasmtime `Store` — is the backends' job. That separation makes policy logic
testable without spawning anything.

## Install

```toml
# Cargo.toml
[dependencies]
tpt-runtime-policy = "0.1"
```

## Policies

| Policy | Behaviour on an unsatisfiable request |
| --- | --- |
| `HardLimit` (default) | deny |
| `SoftLimit` | admit, clamped to what is available |
| `Reservation` | deny (acts as a hard limit in the MVP) |
| `Priority` | admit (advisory in the MVP) |
| `BestEffort` | admit whenever anything remains |

## Admitting a workload

```rust
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::resources::{Memory, ResourceSpec};
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_policy::{HostCapacity, PolicyDecision, PolicyEngine};

let capacity = HostCapacity {
    cpu_cores: 8.0,
    memory: Memory::gib(16),
    gpus: 1,
};
let engine = PolicyEngine::new(capacity);

let spec = WorkloadSpec {
    name: "trainer".to_owned(),
    execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "trainer.exe".to_owned(),
        ..Default::default()
    }),
    resources: ResourceSpec {
        cpu: Some(4.0),
        memory: Some(Memory::gib(4)),
        ..Default::default()
    },
    network: Default::default(),
    volumes: Vec::new(),
    devices: Vec::new(),
    capabilities: Vec::new(),
    labels: Default::default(),
};

match engine.admit(&spec).unwrap() {
    PolicyDecision::Admitted => println!("admitted as asked"),
    PolicyDecision::Adjusted(notes) => println!("admitted with adjustments: {notes:?}"),
    PolicyDecision::Denied(reason) => println!("denied: {reason}"),
}
```

## Hard limits deny explicitly

The default policy refuses rather than silently giving less than was asked.
The reason string names both the request and the capacity, so an operator can
see why.

```rust
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::resources::{Memory, ResourceSpec};
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_policy::{HostCapacity, PolicyDecision, PolicyEngine};

let engine = PolicyEngine::new(HostCapacity {
    cpu_cores: 2.0,
    memory: Memory::gib(4),
    gpus: 0,
});

let spec = WorkloadSpec {
    name: "huge".to_owned(),
    execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "huge.exe".to_owned(),
        ..Default::default()
    }),
    resources: ResourceSpec {
        cpu: Some(64.0),                 // far beyond the 2 cores available
        memory: Some(Memory::gib(4)),
        ..Default::default()
    },
    network: Default::default(),
    volumes: Vec::new(),
    devices: Vec::new(),
    capabilities: Vec::new(),
    labels: Default::default(),
};

match engine.admit(&spec).unwrap() {
    PolicyDecision::Denied(reason) => {
        assert!(reason.contains("64"), "reason names the request: {reason}");
    }
    other => panic!("expected a denial, got {other:?}"),
}
```

`is_admitted` is the convenience predicate when only the outcome matters:

```rust
use tpt_runtime_policy::PolicyDecision;

assert!(!PolicyDecision::Denied("out of memory".to_owned()).is_admitted());
assert!(PolicyDecision::Admitted.is_admitted());
assert!(PolicyDecision::Adjusted(vec!["cpu clamped to 2.0".to_owned()]).is_admitted());
```

## Soft limits clamp instead of denying

`with_default_policy` changes how the engine reacts. Under `SoftLimit` the
request is admitted with an explanation of what was reduced.

```rust
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::resources::{Memory, ResourceSpec};
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_policy::{HostCapacity, PolicyDecision, PolicyEngine, ResourcePolicy};

let engine = PolicyEngine::new(HostCapacity {
    cpu_cores: 2.0,
    memory: Memory::gib(4),
    gpus: 0,
})
.with_default_policy(ResourcePolicy::SoftLimit);

let spec = WorkloadSpec {
    name: "greedy".to_owned(),
    execution: ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "greedy.exe".to_owned(),
        ..Default::default()
    }),
    resources: ResourceSpec {
        cpu: Some(64.0),
        memory: Some(Memory::gib(4)),
        ..Default::default()
    },
    network: Default::default(),
    volumes: Vec::new(),
    devices: Vec::new(),
    capabilities: Vec::new(),
    labels: Default::default(),
};

match engine.admit(&spec).unwrap() {
    PolicyDecision::Adjusted(notes) => {
        assert!(notes.iter().any(|n| n.contains("cpu clamped")));
    }
    other => panic!("expected an adjustment, got {other:?}"),
}
```

## GPU presence checks

A workload that requests a device is denied when the host reports none, so the
failure is an explicit admission error rather than a confusing runtime fault
later (SPEC §48).

## Enforcing a decision

`enforce` is the convenience form of `admit` that turns a denial into a
`ResourceExhausted` error directly:

```rust
use tpt_runtime_model::ExecutionSpec;
use tpt_runtime_model::execution::WindowsProcessSpec;
use tpt_runtime_model::WorkloadSpec;
use tpt_runtime_policy::{HostCapacity, PolicyEngine};

let engine = PolicyEngine::new(HostCapacity::unknown());

// A trivial request is admitted, so enforce succeeds.
let ok = WorkloadSpec::new(
    "tiny",
    ExecutionSpec::WindowsProcess(WindowsProcessSpec {
        program: "tiny.exe".to_owned(),
        ..Default::default()
    }),
);
assert!(engine.enforce(&ok).is_ok());
```

## Unknown capacity

`HostCapacity::unknown` reports effectively unlimited CPU and memory with zero
GPUs. Use it when discovery is unavailable, so policy does not deny workloads
for reasons you cannot substantiate.

## Testing

```console
cargo test -p tpt-runtime-policy
```

Tests cover each policy against satisfied and unsatisfiable CPU, memory and GPU
requests, and the `is_admitted` predicate.

## License

MIT OR Apache-2.0. Copyright (c) 2026 TPT Solutions.

