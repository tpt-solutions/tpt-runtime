//! Structured runtime events (SPEC §28).
//!
//! Events are emitted as JSON with a stable dotted name so that they can be
//! streamed to CLI clients, persisted to a JSONL log, and normalized into
//! backend-specific telemetry later (SPEC §27).

use crate::id::WorkloadId;
use crate::timestamp::Timestamp;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The event names defined by SPEC §28.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    // workload lifecycle
    WorkloadCreated,
    WorkloadResolved,
    WorkloadPrepared,
    WorkloadStarted,
    WorkloadPaused,
    WorkloadResumed,
    WorkloadStopped,
    WorkloadFailed,
    WorkloadDestroyed,
    // resources
    ResourceGranted,
    ResourceDenied,
    ResourceExhausted,
    // capabilities
    CapabilityGranted,
    CapabilityDenied,
    CapabilityUsed,
    // devices
    DeviceAttached,
    DeviceDetached,
    // networking
    NetworkConnected,
    NetworkDisconnected,
}

impl EventKind {
    /// Parses the dotted event name (`workload.started`).
    pub fn parse_name(name: &str) -> Option<Self> {
        let kind = match name {
            "workload.created" => EventKind::WorkloadCreated,
            "workload.resolved" => EventKind::WorkloadResolved,
            "workload.prepared" => EventKind::WorkloadPrepared,
            "workload.started" => EventKind::WorkloadStarted,
            "workload.paused" => EventKind::WorkloadPaused,
            "workload.resumed" => EventKind::WorkloadResumed,
            "workload.stopped" => EventKind::WorkloadStopped,
            "workload.failed" => EventKind::WorkloadFailed,
            "workload.destroyed" => EventKind::WorkloadDestroyed,
            "resource.granted" => EventKind::ResourceGranted,
            "resource.denied" => EventKind::ResourceDenied,
            "resource.exhausted" => EventKind::ResourceExhausted,
            "capability.granted" => EventKind::CapabilityGranted,
            "capability.denied" => EventKind::CapabilityDenied,
            "capability.used" => EventKind::CapabilityUsed,
            "device.attached" => EventKind::DeviceAttached,
            "device.detached" => EventKind::DeviceDetached,
            "network.connected" => EventKind::NetworkConnected,
            "network.disconnected" => EventKind::NetworkDisconnected,
            _ => return None,
        };
        Some(kind)
    }

    /// The dotted event name as it appears on the wire (`workload.started`).
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::WorkloadCreated => "workload.created",
            EventKind::WorkloadResolved => "workload.resolved",
            EventKind::WorkloadPrepared => "workload.prepared",
            EventKind::WorkloadStarted => "workload.started",
            EventKind::WorkloadPaused => "workload.paused",
            EventKind::WorkloadResumed => "workload.resumed",
            EventKind::WorkloadStopped => "workload.stopped",
            EventKind::WorkloadFailed => "workload.failed",
            EventKind::WorkloadDestroyed => "workload.destroyed",
            EventKind::ResourceGranted => "resource.granted",
            EventKind::ResourceDenied => "resource.denied",
            EventKind::ResourceExhausted => "resource.exhausted",
            EventKind::CapabilityGranted => "capability.granted",
            EventKind::CapabilityDenied => "capability.denied",
            EventKind::CapabilityUsed => "capability.used",
            EventKind::DeviceAttached => "device.attached",
            EventKind::DeviceDetached => "device.detached",
            EventKind::NetworkConnected => "network.connected",
            EventKind::NetworkDisconnected => "network.disconnected",
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for EventKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        EventKind::parse_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown event name: {name}")))
    }
}

/// One structured runtime event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    /// Event name, e.g. `workload.started`.
    pub event: EventKind,
    /// Workload the event belongs to, when scoped to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<WorkloadId>,
    /// Backend that produced the event, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// When the event was produced.
    pub timestamp: Timestamp,
    /// Additional event payload (`exit_code`, `reason`, `limit`, ...).
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub fields: Option<serde_json::Map<String, serde_json::Value>>,
}

impl RuntimeEvent {
    /// Creates an event stamped with the current time.
    pub fn now(kind: EventKind) -> Self {
        Self {
            event: kind,
            workload: None,
            backend: None,
            timestamp: Timestamp::now(),
            fields: None,
        }
    }

    /// Attaches workload attribution.
    pub fn with_workload(mut self, workload: impl Into<WorkloadId>) -> Self {
        self.workload = Some(workload.into());
        self
    }

    /// Attaches backend attribution.
    pub fn with_backend(mut self, backend: impl Into<String>) -> Self {
        self.backend = Some(backend.into());
        self
    }

    /// Attaches one structured payload field.
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.fields
            .get_or_insert_with(serde_json::Map::new)
            .insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_like_spec_example() {
        let event = RuntimeEvent::now(EventKind::WorkloadStarted)
            .with_workload("wl-dev-agent")
            .with_backend("linux");
        let value: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["event"], "workload.started");
        assert_eq!(value["workload"], "wl-dev-agent");
        assert_eq!(value["backend"], "linux");
        assert!(value["timestamp"].is_string());
    }

    #[test]
    fn payload_fields_are_flattened() {
        let event = RuntimeEvent::now(EventKind::WorkloadStopped)
            .with_field("exit_code", 0)
            .with_field("reason", "requested");
        let value: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["reason"], "requested");
        assert!(value.get("fields").is_none());
    }

    #[test]
    fn kind_names_match_spec() {
        for (kind, name) in [
            (EventKind::WorkloadFailed, "workload.failed"),
            (EventKind::ResourceExhausted, "resource.exhausted"),
            (EventKind::CapabilityDenied, "capability.denied"),
            (EventKind::DeviceAttached, "device.attached"),
            (EventKind::NetworkConnected, "network.connected"),
        ] {
            assert_eq!(kind.as_str(), name);
            assert_eq!(kind.to_string(), name);
        }
    }
}
