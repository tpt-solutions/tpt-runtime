//! # tpt-runtime-api
//!
//! The stable local API (SPEC §30): a named pipe (Windows) carrying
//! newline-delimited JSON — [`Request`] in, [`Response`] or
//! [`RuntimeEvent`](tpt_runtime_core::event::RuntimeEvent) out.
//!
//! The CLI is a client of this API, never a second runtime (SPEC §30).
//!
//! Method surface (see [`dispatch`]): `workloads.*`, `events.*`,
//! `volumes.*`, `devices.*`, `secrets.*`, `daemon.*`.

pub mod client;
pub mod server;

pub use client::ApiClient;
pub use server::{serve, ApiState};
