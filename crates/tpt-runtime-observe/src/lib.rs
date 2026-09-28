//! # tpt-runtime-observe
//!
//! Observability built in, not bolted on (SPEC §27): structured
//! [`RuntimeEvent`]s broadcast to subscribers and persisted as JSONL, plus
//! per-workload usage snapshots ([`MetricsRegistry`], SPEC §25).
//!
//! ```
//! use tpt_runtime_core::{EventKind, RuntimeEvent};
//! use tpt_runtime_observe::EventHub;
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let hub = EventHub::new(Some(std::env::temp_dir().join("events-test.jsonl")));
//! let mut sub = hub.subscribe();
//! hub.emit(RuntimeEvent::now(EventKind::WorkloadStarted));
//! assert_eq!(sub.recv().await.unwrap().event, EventKind::WorkloadStarted);
//! # });
//! ```

pub mod events;
pub mod metrics;

pub use events::EventHub;
pub use metrics::MetricsRegistry;
