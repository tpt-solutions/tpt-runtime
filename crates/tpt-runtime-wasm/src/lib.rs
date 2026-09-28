//! # tpt-runtime-wasm
//!
//! WASM execution (SPEC §14) on wasmtime with WASI preview 1:
//!
//! - **Sandbox by default**: the module sees only what is granted —
//!   explicitly preopened volumes (filesystem capabilities), a small env,
//!   and no sockets unless a future host extension grants them (SPEC §5.2).
//! - **Resource limits**: deterministic CPU bounding via wasmtime *fuel*
//!   (`resources.fuel`), wall-clock bounding via *epoch* interruption
//!   (`resources.timeout_secs`), and stop-any-time epoch trap.
//! - **Capture**: WASI stdout/stderr flow into the runtime's log buffers
//!   and files like every other backend.
//!
//! Runtime classes (SPEC §14): `command` runs `_start`; `service` /
//! `function` map onto long-running/_invoked exports in a later phase —
//! they currently run `_start` as well, with epoch-based stop.

pub mod backend;

pub use backend::WasmBackend;
