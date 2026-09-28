//! Event hub: broadcast to live subscribers, JSONL persistence
//! (SPEC §28).

use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::event::RuntimeEvent;

/// Broadcasts runtime events to subscribers and appends them to a JSONL
/// file when one is configured. Telemetry stays cheap when nobody is
/// subscribed and no sink is set (SPEC §47).
pub struct EventHub {
    tx: broadcast::Sender<RuntimeEvent>,
    sink: Option<PathBuf>,
}

impl EventHub {
    /// Creates a hub, optionally persisting to `sink` (JSONL).
    pub fn new(sink: Option<impl AsRef<Path>>) -> Self {
        let (tx, _) = broadcast::channel(1024);
        let sink = sink.map(|p| p.as_ref().to_path_buf());
        if let Some(path) = &sink {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        Self { tx, sink }
    }

    /// Registers a live subscriber. The subscriber receives events emitted
    /// after this call; a slow subscriber drops old events rather than
    /// blocking the runtime (broadcast semantics).
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }

    /// Emits one event: appends to the sink (best effort) and broadcasts.
    pub fn emit(&self, event: RuntimeEvent) {
        if let Some(sink) = &self.sink {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(sink)
            {
                use std::io::Write;
                if let Ok(line) = serde_json::to_string(&event) {
                    let _ = writeln!(file, "{line}");
                }
            }
        }
        let _ = self.tx.send(event); // no subscribers: fine
    }

    /// Reads persisted events, newest last. Used by `tpt events` history.
    pub fn read_history(&self, limit: usize) -> Result<Vec<RuntimeEvent>> {
        let Some(sink) = &self.sink else {
            return Ok(Vec::new());
        };
        let raw = std::fs::read_to_string(sink).map_err(|err| {
            RuntimeError::new(
                ErrorKind::NotFound,
                format!("cannot read event log {}: {err}", sink.display()),
            )
        })?;
        let mut events = Vec::new();
        for line in raw.lines().rev() {
            if events.len() == limit {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(line) {
                Ok(event) => events.push(event),
                Err(_) => continue, // torn final line etc.
            }
        }
        events.reverse();
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_runtime_core::id::WorkloadId;
    use tpt_runtime_core::{EventKind, Timestamp};

    fn temp_sink() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "tpt-events-{}-{}.jsonl",
            std::process::id(),
            unique
        ))
    }

    #[tokio::test]
    async fn subscribers_receive_emitted_events() {
        let hub = EventHub::new(None::<PathBuf>);
        let mut sub = hub.subscribe();
        hub.emit(
            RuntimeEvent::now(EventKind::WorkloadStarted)
                .with_workload(WorkloadId::from_raw("wl-1")),
        );
        let received = sub.recv().await.unwrap();
        assert_eq!(received.event, EventKind::WorkloadStarted);
        assert_eq!(received.workload.as_ref().map(|w| w.as_str()), Some("wl-1"));
    }

    #[tokio::test]
    async fn events_persist_and_replay() {
        let sink = temp_sink();
        let hub = EventHub::new(Some(&sink));
        hub.emit(RuntimeEvent::now(EventKind::WorkloadCreated));
        hub.emit(RuntimeEvent::now(EventKind::WorkloadStarted));
        hub.emit(RuntimeEvent::now(EventKind::WorkloadStopped));

        let history = hub.read_history(10).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].event, EventKind::WorkloadCreated);
        assert_eq!(history[2].event, EventKind::WorkloadStopped);
        std::fs::remove_file(&sink).ok();
    }

    #[tokio::test]
    async fn history_limit_returns_newest() {
        let sink = temp_sink();
        let hub = EventHub::new(Some(&sink));
        for i in 0..5 {
            hub.emit(RuntimeEvent::now(EventKind::WorkloadStarted).with_field("i", i));
        }
        let history = hub.read_history(2).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].fields.as_ref().unwrap()["i"], 4);
        std::fs::remove_file(&sink).ok();
    }
}
