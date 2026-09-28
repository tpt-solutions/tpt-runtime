//! Execution backend selection (SPEC §10).

use serde::{Deserialize, Serialize};
use std::fmt;

/// The execution mechanisms tpt-runtime knows about.
///
/// The initial set matches SPEC §10 (`windows`, `linux`, `oci`, `wasm`);
/// future kinds (`microvm`, `remote`, `edge`, `tpt-native`) must be added
/// here without changing the workload model itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// Native Windows processes.
    #[default]
    Windows,
    /// Linux execution environments (WSL-backed today, TPT-managed later).
    Linux,
    /// OCI images executed through Boxcar-compatible primitives (SPEC §13).
    Oci,
    /// WebAssembly modules (SPEC §14).
    Wasm,
}

impl BackendKind {
    /// Canonical lowercase name used in manifests and events.
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::Windows => "windows",
            BackendKind::Linux => "linux",
            BackendKind::Oci => "oci",
            BackendKind::Wasm => "wasm",
        }
    }
}

impl std::str::FromStr for BackendKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "windows" | "win32" => Ok(BackendKind::Windows),
            "linux" => Ok(BackendKind::Linux),
            "oci" | "container" => Ok(BackendKind::Oci),
            "wasm" | "webassembly" => Ok(BackendKind::Wasm),
            other => Err(format!(
                "unknown backend '{other}' (expected windows, linux, oci or wasm)"
            )),
        }
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_spec_backends() {
        for (text, expected) in [
            ("windows", BackendKind::Windows),
            ("linux", BackendKind::Linux),
            ("oci", BackendKind::Oci),
            ("wasm", BackendKind::Wasm),
        ] {
            assert_eq!(text.parse::<BackendKind>().unwrap(), expected);
        }
    }

    #[test]
    fn rejects_unknown_backend() {
        assert!("hyperv".parse::<BackendKind>().is_err());
    }

    #[test]
    fn serializes_as_lowercase() {
        assert_eq!(serde_json::to_string(&BackendKind::Wasm).unwrap(), "\"wasm\"");
    }
}
