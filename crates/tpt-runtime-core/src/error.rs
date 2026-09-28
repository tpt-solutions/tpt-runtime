//! Runtime error type (SPEC §8, §48).

use crate::id::WorkloadId;
use serde::{Deserialize, Serialize};

/// Result alias used across tpt-runtime crates.
pub type Result<T, E = RuntimeError> = std::result::Result<T, E>;

/// Failure categories required by SPEC §48: every significant failure should
/// identify the workload, the operation, the backend and the cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Manifest or configuration is invalid.
    InvalidConfiguration,
    /// The requested lifecycle transition is not allowed.
    InvalidTransition,
    /// The referenced workload or resource does not exist.
    NotFound,
    /// A capability was requested but not granted (SPEC §23).
    CapabilityDenied,
    /// A resource limit could not be satisfied (SPEC §25).
    ResourceExhausted,
    /// The requested backend is not available on this host.
    BackendUnavailable,
    /// The backend is recognized but not implemented yet.
    NotImplemented,
    /// Storage/volume failure (SPEC §45 failure tests).
    StorageFailure,
    /// Network failure (SPEC §45 failure tests).
    NetworkFailure,
    /// Device was requested but disappeared or is unavailable.
    DeviceUnavailable,
    /// Underlying operating system call failed.
    System,
    /// Anything else; carries the message verbatim.
    Other,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ErrorKind {
    type Err = RuntimeError;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "invalid_configuration" => Ok(ErrorKind::InvalidConfiguration),
            "invalid_transition" => Ok(ErrorKind::InvalidTransition),
            "not_found" => Ok(ErrorKind::NotFound),
            "capability_denied" => Ok(ErrorKind::CapabilityDenied),
            "resource_exhausted" => Ok(ErrorKind::ResourceExhausted),
            "backend_unavailable" => Ok(ErrorKind::BackendUnavailable),
            "not_implemented" => Ok(ErrorKind::NotImplemented),
            "storage_failure" => Ok(ErrorKind::StorageFailure),
            "network_failure" => Ok(ErrorKind::NetworkFailure),
            "device_unavailable" => Ok(ErrorKind::DeviceUnavailable),
            "system" => Ok(ErrorKind::System),
            _ => Ok(ErrorKind::Other),
        }
    }
}

impl ErrorKind {
    fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::InvalidConfiguration => "invalid_configuration",
            ErrorKind::InvalidTransition => "invalid_transition",
            ErrorKind::NotFound => "not_found",
            ErrorKind::CapabilityDenied => "capability_denied",
            ErrorKind::ResourceExhausted => "resource_exhausted",
            ErrorKind::BackendUnavailable => "backend_unavailable",
            ErrorKind::NotImplemented => "not_implemented",
            ErrorKind::StorageFailure => "storage_failure",
            ErrorKind::NetworkFailure => "network_failure",
            ErrorKind::DeviceUnavailable => "device_unavailable",
            ErrorKind::System => "system",
            ErrorKind::Other => "other",
        }
    }
}

/// The canonical runtime error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeError {
    /// Error category.
    pub kind: ErrorKind,
    /// Human-readable cause; never empty.
    pub message: String,
    /// Workload the failure is attributed to, when known (SPEC §48).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<WorkloadId>,
    /// Backend that produced the failure, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Operation that failed, when known (e.g. `start`, `stop`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(workload) = &self.workload {
            write!(f, " [{}]", workload)?;
        }
        if let Some(backend) = &self.backend {
            write!(f, " ({})", backend)?;
        }
        if let Some(operation) = &self.operation {
            write!(f, " during {}", operation)?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for RuntimeError {}

impl RuntimeError {
    /// Creates an error of the given kind.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            workload: None,
            backend: None,
            operation: None,
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

    /// Attaches the failing operation.
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }
}

impl From<std::io::Error> for RuntimeError {
    fn from(err: std::io::Error) -> Self {
        RuntimeError::new(ErrorKind::System, err.to_string())
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(err: serde_json::Error) -> Self {
        RuntimeError::new(ErrorKind::Other, format!("json error: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributions_serialize_compactly() {
        let err = RuntimeError::new(ErrorKind::CapabilityDenied, "network.outbound not granted")
            .with_workload(WorkloadId::from_raw("wl-1"))
            .with_backend("windows")
            .with_operation("start");
        let value: serde_json::Value = serde_json::to_value(&err).unwrap();
        assert_eq!(value["kind"], "capability_denied");
        assert_eq!(value["workload"], "wl-1");
        assert_eq!(value["backend"], "windows");
        assert_eq!(value["operation"], "start");
    }

    #[test]
    fn display_includes_workload_when_present() {
        let err = RuntimeError::new(ErrorKind::NotFound, "no such workload")
            .with_workload(WorkloadId::from_raw("wl-x"));
        assert!(err.to_string().contains("wl-x"));
    }
}
