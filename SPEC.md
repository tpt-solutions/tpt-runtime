TPT Runtime — Design Specification

Project: tpt-runtime
Organization: TPT Solutions
Status: Architecture / Design Specification
License: MIT OR Apache-2.0
Primary language: Rust
Target host: Windows initially
Target workloads: Windows, Linux, OCI, WASM, native TPT workloads
Related projects: tpt-archon, tpt-boxcar, tpt-infer, tpt-dsp, tpt-orchestra

1. Executive Summary

tpt-runtime is a unified workload runtime for Windows designed to provide a coherent execution, storage, networking, security, device, and observability environment for heterogeneous workloads.

The project is not intended to be a direct WSL clone.

Its purpose is to provide a common runtime substrate in which:

Linux workloads can execute on Windows.

OCI containers can execute without requiring a separate user-facing runtime model.

WASM workloads can execute as first-class workloads.

Native Windows processes can participate in the same workload environment.

TPT-native workloads can use shared runtime primitives.

Storage, networking, devices, security, lifecycle, and observability are managed consistently.

AI workloads can be treated as ordinary runtime workloads.

Workloads can eventually move between Windows, Linux, edge, and other TPT environments with minimal changes.

The central architectural principle is:

Do not make Linux pretend to be Windows or Windows pretend to be Linux. Provide a common workload substrate underneath both.

tpt-runtime should therefore be designed as a runtime/orchestration layer above lower-level substrate components such as tpt-archon, while tpt-boxcar provides workload isolation, packaging, networking, service, and sandbox capabilities.

2. Problem Statement

Modern Windows development environments commonly require a collection of loosely integrated systems:

Windows
├── Windows processes
├── WSL
│   └── Linux kernel
├── Docker / OCI runtime
├── VM infrastructure
├── WASM runtime
├── GPU runtime
├── local AI runtime
├── databases
└── development toolchains

Each system has its own:

filesystem semantics

process model

networking model

lifecycle

security boundary

configuration

observability

resource accounting

caching

device integration

This produces unnecessary boundaries.

Typical examples include:

Windows filesystem
        ↓
WSL filesystem bridge
        ↓
Linux filesystem

or:

Windows network
        ↓
virtual network
        ↓
Linux network
        ↓
container network

or:

Windows GPU
        ↓
virtualized interface
        ↓
Linux GPU stack
        ↓
AI runtime

tpt-runtime seeks to provide a common resource and workload model without requiring every workload type to use identical implementation mechanisms.

3. Goals

3.1 Primary Goals

Unified workload model

Provide one conceptual model for:

native processes

Linux workloads

OCI containers

WASM workloads

sandboxed workloads

AI workloads

TPT-native workloads

Unified lifecycle

Every workload should have a consistent lifecycle:

defined
  ↓
resolved
  ↓
prepared
  ↓
created
  ↓
started
  ↓
running
  ↓
paused
  ↓
stopped
  ↓
destroyed

Unified resource model

Provide consistent concepts for:

CPU

memory

storage

networking

devices

GPU

IPC

secrets

capabilities

Strong isolation

Workloads must receive only the resources and capabilities explicitly granted to them.

Windows-first

Windows is the initial host platform.

The architecture must not, however, embed Windows-specific assumptions into the core runtime model.

Rust-first

The runtime should be implemented primarily in Rust.

Unsafe Rust should be isolated to:

OS interfaces

device interfaces

virtualization interfaces

SIMD

FFI

unavoidable low-level primitives

Composability

TPT projects should be able to reuse runtime primitives as libraries and services.

4. Non-Goals

tpt-runtime is not initially intended to:

replace the Windows kernel

implement a new general-purpose operating system

reimplement the Linux kernel

become a complete hypervisor

replace every existing container runtime immediately

replace Windows itself

require every workload to run inside a VM

require every workload to be WASM

provide Kubernetes compatibility as its primary abstraction

become a general cloud orchestration platform

Compatibility should be achieved where useful without allowing compatibility requirements to dictate the architecture.

5. Design Principles

5.1 Workloads, not operating systems

The primary abstraction is the workload.

A workload may happen to contain:

a Linux environment

a Windows process

an OCI image

a WASM module

a TPT-native executable

The runtime should care primarily about:

What does this workload require, and what resources is it allowed to access?

5.2 Capabilities over ambient authority

Workloads should not automatically inherit host access.

Example:

workload
 ├── filesystem: project-readwrite
 ├── network: outbound
 ├── GPU: none
 ├── camera: none
 ├── secret: github-token
 └── IPC: service:database

Capabilities should be explicit, inspectable, revocable where practical, and auditable.

5.3 One resource model, multiple execution mechanisms

Linux, Windows, OCI, and WASM do not need identical implementations.

They should expose common runtime concepts:

Workload
Resource
Capability
Volume
Network
Device
Service
Identity
Policy
Lifecycle

5.4 Zero-copy where practical

Where data crosses runtime boundaries, avoid unnecessary copies.

This is particularly important for:

file I/O

networking

media

AI inference

GPU workloads

IPC

large tensors

tpt-archon should provide lower-level primitives where appropriate.

5.5 Observable by default

Every workload should have structured information available about:

CPU

memory

I/O

network

storage

process state

lifecycle events

capability usage

failures

resource pressure

5.6 Deterministic configuration

A workload definition should be declarative and reproducible.

Example:

name = "dev-api"
runtime = "oci"

[image]
name = "my-api"
version = "1.4.0"

[resources]
cpu = 4
memory = "4GiB"

[network]
mode = "service"

[[volumes]]
name = "source"
mount = "/workspace"
mode = "read-write"

6. High-Level Architecture

                         Windows Host
                              │
                     ┌────────┴────────┐
                     │   TPT Runtime   │
                     └────────┬────────┘
                              │
       ┌──────────────────────┼──────────────────────┐
       │                      │                      │
 Execution                Resources             Management
       │                      │                      │
 ┌─────┼──────┐        ┌──────┼────────┐       ┌─────┼─────┐
 │     │      │        │      │        │       │     │     │
Win  Linux   WASM   Storage Network Devices  CLI  API  Events
 │     │      │        │      │        │
 │    OCI     │        │      │        │
 │     │      │        └──────┼────────┘
 └─────┴──────┘               │
                         TPT Archon
                              │
                    Host / kernel interfaces

tpt-boxcar sits primarily across the workload isolation, packaging, networking, service, and sandbox layers.

7. Component Model

The initial workspace should be designed around clear boundaries.

Suggested crates:

tpt-runtime-core
tpt-runtime-model
tpt-runtime-config
tpt-runtime-policy
tpt-runtime-capability
tpt-runtime-workload
tpt-runtime-process
tpt-runtime-linux
tpt-runtime-oci
tpt-runtime-wasm
tpt-runtime-windows
tpt-runtime-storage
tpt-runtime-network
tpt-runtime-device
tpt-runtime-gpu
tpt-runtime-ipc
tpt-runtime-observe
tpt-runtime-security
tpt-runtime-api
tpt-runtime-cli
tpt-runtime-daemon
tpt-runtime-test

Not all crates need to exist in version 0.1.

The architecture should allow the workspace to grow without creating a monolithic runtime crate.

8. Runtime Core

tpt-runtime-core contains platform-independent runtime primitives.

Responsibilities:

identifiers

lifecycle states

resource identifiers

workload identifiers

errors

timestamps

events

handles

state transitions

Example conceptual types:

WorkloadId
ResourceId
VolumeId
NetworkId
DeviceId
CapabilityId
ServiceId
WorkloadState
RuntimeEvent
RuntimeError

The core crate must avoid unnecessary platform dependencies.

9. Workload Model

A workload consists of:

Workload
├── Identity
├── Execution
├── Resources
├── Capabilities
├── Storage
├── Networking
├── Devices
├── Environment
├── Policies
├── Observability
└── Lifecycle

Example:

Workload: tpt-dev-agent

Execution:
    type = Linux
    image = Ubuntu

Resources:
    CPU = 8
    RAM = 8 GiB
    GPU = RTX 3050

Storage:
    source = project-volume
    cache = enabled

Network:
    outbound = enabled
    inbound = service-only

Capabilities:
    filesystem = project
    git = allowed
    GPU = allowed
    secrets = github

10. Execution Backends

tpt-runtime should define an execution backend interface.

Conceptually:

trait ExecutionBackend {
    fn prepare(&self, workload: &WorkloadSpec) -> Result<PreparedWorkload>;
    fn start(&self, workload: &PreparedWorkload) -> Result<WorkloadHandle>;
    fn stop(&self, handle: &WorkloadHandle) -> Result<()>;
}

Initial backend types:

windows
linux
oci
wasm

Future:

microvm
remote
edge
tpt-native

The workload model must remain independent of the backend.

11. Linux Backend

Linux support is the most strategically important component because it is where the WSL comparison arises.

The first implementation should not attempt to eliminate WSL immediately.

Instead, support a layered strategy:

Phase 1

Use existing Windows/Linux virtualization infrastructure where appropriate.

Phase 2

Integrate more directly with TPT resource management.

Phase 3

Investigate a TPT-managed Linux execution environment.

The runtime must distinguish:

Linux compatibility

from:

Linux implementation

A workload only requires a Linux-compatible execution environment. The implementation can evolve independently.

12. Windows Backend

Windows workloads should be first-class.

The runtime should support:

native processes

Windows services where appropriate

Windows process groups

environment management

resource accounting

capability enforcement where supported

lifecycle control

stdout/stderr/event capture

Example:

tpt run windows --exe myapp.exe

13. OCI Backend

OCI should be treated as a compatibility format, not the fundamental architecture.

Responsibilities:

image resolution

image storage

layer management

filesystem preparation

process configuration

lifecycle

resource assignment

networking

isolation

tpt-boxcar should provide reusable primitives wherever appropriate.

The runtime should avoid assuming that every workload must be packaged as an OCI image.

14. WASM Backend

WASM should be a first-class execution target.

Advantages:

strong sandboxing

portable execution

small deployment units

deterministic resource boundaries

edge compatibility

service workloads

Potential runtime classes:

wasm-command
wasm-service
wasm-function
wasm-filter
wasm-plugin

tpt-boxcar should provide reusable WASM sandbox/service functionality.

15. Storage Architecture

Storage is one of the areas where tpt-archon can provide a major architectural advantage.

The runtime should expose logical volumes:

Volume
├── identity
├── backing store
├── cache policy
├── permissions
├── snapshots
├── sharing policy
└── lifecycle

Example:

tpt volume create project
tpt volume mount project /workspace

A volume may have multiple workload views.

Conceptually:

                  TPT Volume
                       │
          ┌────────────┼────────────┐
          │            │            │
       Windows       Linux        WASM
        view          view         view

The implementation should avoid requiring expensive filesystem translation whenever a common storage layer can safely provide the required semantics.

16. Archon Integration

tpt-archon should initially be treated as a substrate dependency rather than being tightly coupled to the entire runtime.

Potential integration areas:

page/cache management

storage

IPC

capability primitives

device abstraction

zero-copy buffers

I/O

resource accounting

Architecture:

tpt-runtime
      │
      ├── workload manager
      ├── policy
      ├── lifecycle
      └── resource manager
              │
           tpt-archon
              │
      ┌───────┼────────┐
      │       │        │
   storage   IPC     devices

The runtime should not duplicate Archon primitives unnecessarily.

17. Boxcar Integration

tpt-boxcar should provide workload-level functionality.

Potential integration:

tpt-runtime
      │
      ├── workload
      │
      ├── sandbox
      │
      ├── networking
      │
      ├── services
      │
      └── image lifecycle
              │
           tpt-boxcar

Boxcar components should remain independently usable where practical.

18. Networking

The runtime should provide a logical network abstraction.

Workloads should be able to request:

none
host
private
service
isolated
outbound

Instead of requiring users to understand host interfaces, virtual adapters, NAT, bridges, and namespaces for ordinary workloads.

Example:

[network]
mode = "service"

[network.expose]
http = 8080

The runtime resolves the underlying networking mechanism.

tpt-boxcar service mesh/API gateway components can provide higher-level service connectivity.

19. Device Model

Devices should be capabilities.

Examples:

device:gpu:0
device:camera:0
device:audio:0
device:usb:abc
device:serial:com3

A workload requests access:

[[devices]]
id = "gpu:0"
mode = "compute"

The runtime decides how the access is implemented.

This allows the same workload model to eventually support:

Windows
Linux
edge
remote

without exposing platform-specific details in workload definitions.

20. GPU Architecture

GPU access should be treated as a special device class.

Requirements:

device discovery

capability grants

resource accounting

process isolation where supported

memory accounting where possible

telemetry

future partitioning

Initial target:

Windows host
+
NVIDIA GPU
+
Linux/AI workload

This is particularly relevant to tpt-infer.

The runtime should eventually make:

tpt run --gpu qwen

a normal workload operation rather than requiring users to manually configure several layers.

21. AI Workloads

AI workloads should not require a separate architecture.

Examples:

Ollama
TPT Infer
ONNX runtime
WASM inference
GPU-native inference

They are workloads with special resource requirements.

Example:

[resources]
memory = "8GiB"

[gpu]
required = true
memory = "6GiB"

[storage]
cache = "models"

This allows the runtime to make AI infrastructure observable and schedulable using the same primitives as everything else.

22. IPC

The runtime should provide common IPC abstractions:

request/response
streams
shared memory
message queues
event channels
zero-copy buffers

The implementation may use:

Windows primitives

Linux primitives

sockets

shared memory

Archon IPC

Applications should not need to know which mechanism is being used when the abstraction is sufficient.

23. Security Model

Security should be capability-oriented.

Each workload receives:

Identity
+
Capabilities
+
Resources
+
Policy

Example:

Workload: build-agent

Allowed:
    project.read
    project.write
    git.network
    package.network
    compiler.execute

Denied:
    host.filesystem
    camera
    microphone
    arbitrary.devices

Policies should be declarative.

Every privileged operation should ideally be attributable to:

workload
→ identity
→ capability
→ resource
→ operation

24. Secrets

Secrets should never be treated as ordinary environment variables by default.

The runtime should eventually support:

tpt secret create github-token
tpt secret grant github-token build-agent

A workload receives an ephemeral capability rather than unrestricted host access.

25. Resource Management

The runtime should track:

CPU
RAM
storage
IOPS
network bandwidth
GPU
GPU memory
process count
handles
devices

Resource policies may be:

hard limit
soft limit
reservation
priority
best effort

The runtime should expose both requested and actual usage.

26. Lifecycle Management

Workloads should support:

create
start
pause
resume
restart
stop
kill
snapshot
restore
destroy

Lifecycle events should be persisted or streamed through the runtime event system.

27. Observability

Observability should be built into the runtime rather than bolted on.

Required data:

workload state
CPU
memory
I/O
network
GPU
filesystem activity
capability requests
errors
startup time
shutdown time
resource pressure

tpt-boxcar eBPF functionality should be considered for Linux observability.

Windows ETW and native telemetry mechanisms should be integrated for Windows workloads.

The runtime should normalize these into common events where possible.

28. Event Model

Example:

{
  "event": "workload.started",
  "workload": "dev-agent",
  "timestamp": "...",
  "backend": "linux"
}

Other events:

workload.created
workload.prepared
workload.started
workload.paused
workload.resumed
workload.stopped
workload.failed

resource.granted
resource.denied
resource.exhausted

capability.granted
capability.denied
capability.used

device.attached
device.detached

network.connected
network.disconnected

29. CLI

The CLI should be the primary developer interface initially.

Examples:

tpt runtime status

tpt list

tpt run ubuntu

tpt run --oci postgres

tpt run --wasm service.wasm

tpt run --gpu tpt-infer

tpt inspect workload-id

tpt logs workload-id

tpt stop workload-id

tpt restart workload-id

Resource commands:

tpt volume list
tpt network list
tpt device list
tpt gpu list

Security:

tpt capability list
tpt policy inspect workload-id

30. API

The runtime daemon should expose a stable local API.

Potential transports:

Windows named pipes
Unix sockets where applicable
TCP/TLS for controlled remote operation

The API should expose:

workloads
resources
volumes
networks
devices
capabilities
events
logs
metrics

The CLI should be a client of this API rather than embedding runtime logic.

31. Configuration

Configuration should be declarative.

Possible formats:

TOML
YAML
JSON

TOML should be the default for human-authored configuration.

Example:

name = "developer"

[runtime]
backend = "linux"

[resources]
cpu = 8
memory = "8GiB"

[network]
mode = "outbound"

[storage]
project = "C:/Projects/example"

[gpu]
enabled = true

32. Workload Manifest

A standard manifest should eventually be defined.

Suggested structure:

api = "tpt.runtime/v1"

[workload]
name = "example"

[execution]
backend = "oci"

[resources]
cpu = 4
memory = "4GiB"

[network]
mode = "service"

[[volumes]]
name = "source"
mount = "/workspace"
mode = "read-write"

[[capabilities]]
name = "network.outbound"

The manifest should be backend-independent where possible.

33. Developer Experience

A major goal is reducing environment setup.

Instead of:

install WSL
install distro
install Docker
configure Docker
configure GPU
configure networking
configure volumes
install toolchain
configure environment

the desired experience is:

tpt init
tpt run dev

The runtime resolves the required environment.

34. Project Environments

tpt-runtime should support project-local environments.

Example:

project/
├── tpt.toml
├── src/
├── Cargo.toml
└── ...

tpt.toml could define:

[environment]
name = "rust-dev"

[toolchain]
rust = "stable"

[services]
database = "postgres"
redis = "redis"

[resources]
memory = "8GiB"

Then:

tpt up

creates the required environment.

This could eventually provide a unified alternative to combinations of:

devcontainers

WSL distributions

Docker Compose

ad-hoc scripts

without requiring those systems to disappear.

35. Compatibility Strategy

Compatibility should be layered.

Tier 1

Native TPT workloads.

Tier 2

WASM.

Tier 3

OCI.

Tier 4

Linux.

Tier 5

Windows.

All should use the same management model.

Compatibility layers should be replaceable.

36. Relationship to WSL

tpt-runtime should initially coexist with WSL.

Possible architecture:

Windows
│
├── WSL
│    └── Linux
│
└── TPT Runtime
     ├── Windows
     ├── OCI
     ├── WASM
     └── Linux

Over time:

Windows
│
└── TPT Runtime
     ├── Windows
     ├── Linux
     ├── OCI
     ├── WASM
     └── TPT-native

The project should only replace underlying mechanisms where measurable improvements justify the complexity.

37. Interoperability

The runtime should support interoperability with existing systems.

Potential integrations:

WSL
Docker / OCI
containerd
WASM runtimes
Git
VS Code
PowerShell
Windows Terminal
NVIDIA CUDA
OpenAI-compatible APIs
Ollama
TPT Infer

Interoperability should be achieved through adapters rather than contaminating the core model.

38. VS Code Integration

Eventually provide a VS Code extension capable of:

detecting tpt.toml

starting project environments

displaying workload state

attaching terminals

viewing logs

viewing resource usage

managing services

selecting execution targets

The extension should communicate through the runtime API.

39. Remote Runtime

The architecture should eventually allow:

local runtime
      │
      └── remote runtime

A workload could theoretically execute on:

desktop
server
NAS
edge device
cloud

without changing its logical definition.

This is a future capability and should not complicate the initial Windows implementation.

40. Edge Runtime

Because Boxcar and other TPT projects target edge environments, the runtime model should remain small enough to eventually run on constrained systems.

A future architecture could be:

TPT Runtime
├── Desktop profile
├── Server profile
├── Edge profile
└── Embedded profile

The same workload model should survive across profiles.

41. Architecture Boundaries

The following boundaries should remain explicit:

tpt-runtime
    orchestration / lifecycle / policy

tpt-archon
    low-level substrate / storage / IPC / resources

tpt-boxcar
    sandbox / workload packaging / networking / services

tpt-infer
    inference

tpt-dsp
    signal processing

tpt-orchestra
    multi-agent / repository orchestration

Avoid turning tpt-runtime into a catch-all repository.

42. Proposed Repository Structure

tpt-runtime/
├── README.md
├── LICENSE-MIT
├── LICENSE-APACHE
├── SPEC.md
├── todo.md
├── Cargo.toml
├── crates/
│   ├── tpt-runtime-core/
│   ├── tpt-runtime-model/
│   ├── tpt-runtime-config/
│   ├── tpt-runtime-policy/
│   ├── tpt-runtime-capability/
│   ├── tpt-runtime-workload/
│   ├── tpt-runtime-process/
│   ├── tpt-runtime-windows/
│   ├── tpt-runtime-linux/
│   ├── tpt-runtime-oci/
│   ├── tpt-runtime-wasm/
│   ├── tpt-runtime-storage/
│   ├── tpt-runtime-network/
│   ├── tpt-runtime-device/
│   ├── tpt-runtime-gpu/
│   ├── tpt-runtime-ipc/
│   ├── tpt-runtime-security/
│   ├── tpt-runtime-observe/
│   ├── tpt-runtime-api/
│   ├── tpt-runtime-daemon/
│   └── tpt-runtime-cli/
├── examples/
├── tests/
└── docs/

43. Initial MVP

The MVP should not attempt to solve all of WSL.

The MVP should prove the architecture.

MVP capabilities

Windows host daemon.

Workload manifest.

Workload lifecycle.

Native Windows process backend.

OCI backend through Boxcar-compatible primitives.

WASM backend.

Logical volume abstraction.

Logical network abstraction.

Capability model.

Resource accounting.

CLI.

Local API.

Structured events.

Basic observability.

Example:

tpt run --wasm hello.wasm
tpt run --oci postgres
tpt run --windows myapp.exe
tpt list
tpt inspect <id>
tpt logs <id>
tpt stop <id>

If this works cleanly, Linux integration becomes the next major backend rather than being the foundation on which the whole project depends.

44. Phase Roadmap

Phase 0 — Architecture

Define workload model.

Define resource model.

Define capability model.

Define lifecycle.

Define API.

Define event model.

Define crate boundaries.

Document Archon integration points.

Document Boxcar integration points.

Phase 1 — Runtime Core

Create workspace.

Implement identifiers.

Implement workload state machine.

Implement manifests.

Implement configuration.

Implement errors.

Implement events.

Phase 2 — Windows Runtime

Runtime daemon.

Windows process backend.

Process lifecycle.

Environment management.

stdout/stderr capture.

Resource accounting.

CLI.

Phase 3 — WASM

WASM backend.

sandbox.

resource limits.

filesystem capabilities.

network capabilities.

service lifecycle.

Phase 4 — OCI

OCI image support.

image cache.

filesystem preparation.

workload lifecycle.

networking.

volumes.

capability integration.

Phase 5 — Archon

Archon storage adapter.

Archon IPC adapter.

shared buffer support.

capability integration.

resource accounting integration.

zero-copy paths where justified.

Phase 6 — Linux

Linux backend abstraction.

WSL integration.

Linux workload lifecycle.

Linux networking.

Linux storage.

Linux observability.

capability translation.

Phase 7 — GPU

GPU discovery.

GPU capability model.

NVIDIA integration.

GPU telemetry.

TPT Infer integration.

GPU resource policies.

Phase 8 — Developer Platform

Project environments.

tpt up.

tpt down.

service dependencies.

VS Code integration.

project templates.

Phase 9 — Advanced Runtime

snapshots.

checkpoint/restore research.

remote runtime.

edge runtime.

workload migration research.

advanced isolation.

runtime optimization.

45. Testing Strategy

Testing must occur at several levels.

Unit tests

Test:

manifests

state transitions

policies

capabilities

resource calculations

identifiers

event serialization

Integration tests

Test:

runtime → Windows process
runtime → WASM
runtime → OCI
runtime → Archon
runtime → Boxcar

Compatibility tests

Test:

Windows
Linux
OCI
WASM
GPU
filesystem
network

Failure tests

Explicitly test:

workload crashes

resource exhaustion

capability denial

device disappearance

network failure

storage failure

runtime restart

host restart

46. Security Testing

Security must include:

capability escalation tests

filesystem escape tests

process isolation tests

network isolation tests

secret leakage tests

device access tests

malformed manifest tests

malicious image tests

WASM sandbox tests

OCI isolation tests

Security boundaries should be tested as executable properties wherever possible.

47. Performance Goals

The runtime should optimize for:

Startup

Small WASM and native workloads should start with minimal overhead.

I/O

Avoid unnecessary copies.

Storage

Use shared caching where safe.

Networking

Avoid unnecessary NAT/translation layers.

AI

Avoid copying model/tensor data unnecessarily between:

storage
RAM
GPU
runtime
inference engine

Observability

Telemetry should impose minimal overhead when detailed tracing is disabled.

48. Failure Philosophy

The runtime should prefer explicit failure over silent degradation.

For example:

GPU capability unavailable

should be distinguishable from:

GPU capability granted
but workload failed to initialize CUDA

Every significant failure should include:

workload
operation
resource
cause
backend
timestamp

49. Versioning

Runtime APIs should be versioned.

Example:

tpt.runtime/v1

Manifest compatibility should be maintained independently from implementation versions.

Crates should follow semantic versioning.

50. Licensing

Default project license:

MIT OR Apache-2.0

Dependencies must be audited for compatibility.

The runtime should avoid architectural dependence on incompatible proprietary components.

Platform APIs may remain proprietary host interfaces while the TPT abstraction layer remains open source.

51. Long-Term Vision

The long-term objective is not merely to provide a better Linux-on-Windows experience.

The objective is to create a general workload substrate.

Conceptually:

                    TPT RUNTIME
                         │
       ┌─────────────────┼─────────────────┐
       │                 │                 │
    Windows             Linux             WASM
       │                 │                 │
       └─────────────────┼─────────────────┘
                         │
                  Common resources
                         │
        ┌────────────────┼────────────────┐
        │                │                │
      Archon           Boxcar          Devices
        │                │                │
     Storage          Sandbox        GPU / I/O
        │                │                │
        └────────────────┼────────────────┘
                         │
                   TPT Workloads
                         │
        ┌────────────────┼────────────────┐
        │                │                │
     Apps             Services            AI

The runtime should make the underlying execution environment increasingly irrelevant to the application.

A developer should eventually be able to think:

“I need this workload to have 4 CPUs, 8 GB RAM, this volume, this network access, this GPU and these capabilities.”

rather than:

“I need Windows + WSL + Docker + a VM + a virtual network + a GPU bridge + a special filesystem setup.”

52. Strategic Positioning

tpt-runtime should be positioned as:

A unified workload runtime for Windows and heterogeneous compute.

Not:

“A WSL replacement.”

WSL compatibility is one use case.

Container execution is another.

WASM is another.

AI is another.

Developer environments are another.

Edge workloads are another.

The common foundation is the workload abstraction.

53. Architectural North Star

The final architecture should make this possible:

tpt run my-workload

with the runtime determining:

What execution environment?
What resources?
What storage?
What network?
What devices?
What capabilities?
What isolation?
What dependencies?
What telemetry?

while still allowing the user to override any decision explicitly.

The runtime should optimize implementation details without hiding important security or resource decisions from the user.

54. First Principle

The project should continuously return to one question:

Can the workload model be made simpler without making the underlying system less capable?

If the answer is yes, simplify the abstraction.

If a feature only exists because an underlying implementation leaks through the abstraction, reconsider the architecture.

tpt-runtime succeeds when developers stop thinking about whether a workload is “inside WSL,” “inside Docker,” “inside WASM,” or “running natively,” and instead think about the workload, its capabilities, and the resources it has been given.