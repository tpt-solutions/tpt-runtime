//! # tpt-runtime-ipc
//!
//! IPC primitives shared by the daemon, API and CLI (SPEC §22, §30):
//!
//! - [`Request`] / [`Response`] — the JSON envelope for the local API.
//! - [`read_message`] / [`write_message`] — newline-delimited JSON framing
//!   over any async byte stream (named pipes on Windows).
//! - Events are [`RuntimeEvent`](tpt_runtime_core::event::RuntimeEvent)
//!   values streamed as their own messages (server → client only).
//!
//! The framing is deliberately simple so that `curl`-style manual probing,
//! the CLI, the VS Code extension (SPEC §38) and future transports can all
//! share one protocol.

pub mod envelope;
pub mod framing;
#[cfg(feature = "archon")]
pub mod shared;

pub use envelope::{Request, Response};
pub use framing::{read_message, write_message};
