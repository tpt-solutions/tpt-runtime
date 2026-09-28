//! The local API JSON envelope (SPEC §30).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A client request: `{"id": 1, "method": "list", "params": {...}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Correlation id echoed in the response; events have no id.
    pub id: u64,
    /// Method name, e.g. `workloads.list`.
    pub method: String,
    /// Method parameters; `null` when the method takes none.
    #[serde(default)]
    pub params: Value,
}

/// A server response to a request: exactly one of `result`/`error`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Correlation id of the answered request.
    pub id: u64,
    /// Success payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl Response {
    /// A successful response.
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response from a runtime error.
    pub fn err(id: u64, error: &tpt_runtime_core::error::RuntimeError) -> Self {
        Self {
            id,
            result: None,
            error: Some(ResponseError::from(error)),
        }
    }

    /// A synthetic error response.
    pub fn error(id: u64, kind: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            result: None,
            error: Some(ResponseError {
                kind: kind.to_owned(),
                message: message.into(),
                workload: None,
                backend: None,
                operation: None,
            }),
        }
    }
}

/// Error payload carried in [`Response::error`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseError {
    /// Error kind (snake_case, mirrors `ErrorKind`).
    pub kind: String,
    /// Human-readable message.
    pub message: String,
    /// Attributed workload, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<String>,
    /// Attributed backend, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Failing operation, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

impl From<&tpt_runtime_core::error::RuntimeError> for ResponseError {
    fn from(err: &tpt_runtime_core::error::RuntimeError) -> Self {
        Self {
            kind: err.kind.to_string(),
            message: err.message.clone(),
            workload: err.workload.as_ref().map(|w| w.to_string()),
            backend: err.backend.clone(),
            operation: err.operation.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_envelope_shape() {
        let req = Request {
            id: 7,
            method: "workloads.list".to_owned(),
            params: Value::Null,
        };
        let value: Value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "workloads.list");
        let back: Request = serde_json::from_value(value).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn response_is_result_or_error() {
        let ok = Response::ok(1, Value::Bool(true));
        let value: Value = serde_json::to_value(&ok).unwrap();
        assert_eq!(value["result"], true);
        assert!(value.get("error").is_none());

        let err = Response::error(2, "not_found", "no such workload");
        let value: Value = serde_json::to_value(&err).unwrap();
        assert_eq!(value["error"]["kind"], "not_found");
        assert!(value.get("result").is_none());
    }
}
