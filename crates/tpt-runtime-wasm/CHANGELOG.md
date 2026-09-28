# Changelog

All notable changes to `tpt-runtime-wasm` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Map the `service` and `function` runtime classes onto long-running and
  invoked exports instead of `_start`.
- Host extensions to grant sockets, currently withheld entirely.
- A shared `tpt-boxcar` WASM service layer so plugins across TPT projects get
  identical sandbox semantics.

## [0.1.0]

Initial release: wasmtime + WASI preview 1 execution.

### Added

- **`WasmBackend`** (`backend`): an `ExecutionBackend` for the `wasm` backend
  kind, with `new` building an engine configured for fuel metering and epoch
  interruption.
- **Fuel-based CPU bounding** via `resources.fuel`, giving deterministic
  instruction budgets.
- **Epoch-based wall-clock bounding** via `resources.timeout_secs`, backed by a
  background ticker incrementing the engine epoch every 10 ms so a
  compute-bound module traps promptly.
- **Stop at any time** through an epoch trap, without waiting for the module to
  reach a trap of its own.
- **Sandbox by default**: only explicitly granted preopened volumes are
  visible, with a minimal environment and no sockets.
- **WASI preview 1** with read-write and read-only preopens mapped from
  `ResolvedMount` access modes.
- **Uniform capture**: WASI stdout/stderr flow into the shared `LogCapture`
  buffers and files.
- Acceptance of both binary `.wasm` and WAT text input, since wasmtime parses
  either.
- `WasmClass` handling for `command`, `service` and `function`; the latter two
  currently run `_start` with epoch-based stop and are documented as such.

[Unreleased]: https://github.com/tpt-solutions/tpt-runtime/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/tpt-solutions/tpt-runtime/releases/tag/v0.1.0
