# TPT Runtime

A unified workload runtime for Windows and heterogeneous compute, providing a coherent execution, storage, networking, security, device, and observability environment for Windows, Linux, OCI, WASM, and TPT-native workloads.

tpt-runtime is not a WSL clone. Its goal is to provide a common workload substrate beneath Windows and Linux execution rather than making one pretend to be the other — so that native processes, containers, WASM modules, and AI workloads can be managed with the same lifecycle, resource, and capability model.

See [SPEC.md](SPEC.md) for the full design specification and [todo.md](todo.md) for the project checklist.

## Status

Architecture / design phase. No implementation yet.

## Related projects

- `tpt-archon` — low-level substrate (storage, IPC, resources)
- `tpt-boxcar` — workload isolation, packaging, networking, sandboxing
- `tpt-infer` — inference
- `tpt-dsp` — signal processing
- `tpt-orchestra` — multi-agent / repository orchestration

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

Copyright © 2026 TPT Solutions.
