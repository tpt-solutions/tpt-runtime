//! Daemon configuration resolved from the environment and defaults.

use std::path::PathBuf;
use tpt_runtime_core::error::Result;

/// Default local API pipe name (SPEC §30: Windows named pipes).
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\tpt-runtime-api";

/// Runtime-wide settings for the host daemon (SPEC §43: Windows host daemon).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    /// Root directory for runtime state (volumes, logs, events, secrets).
    pub state_dir: PathBuf,
    /// Named pipe used by the local API and the CLI.
    pub pipe_name: String,
    /// Maximum concurrent API clients.
    pub max_clients: usize,
    /// TCP listener (`host:port`) for remote access (SPEC §39). When set,
    /// it replaces the named pipe for both server and clients.
    ///
    /// Security: the TCP transport has **no authentication** - it is for
    /// trusted networks only (the named pipe carries Windows ACLs; TCP
    /// carries nothing). Keep it loopback or behind a firewall you trust.
    pub tcp: Option<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            state_dir: default_state_dir(),
            pipe_name: pipe_name_from_env().unwrap_or_else(|| DEFAULT_PIPE_NAME.to_owned()),
            max_clients: 32,
            tcp: tcp_from_env(),
        }
    }
}

impl DaemonConfig {
    /// Resolves configuration from environment overrides:
    /// `TPT_RUNTIME_DIR` (state directory) and `TPT_RUNTIME_PIPE` (pipe name).
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Some(dir) = std::env::var_os("TPT_RUNTIME_DIR") {
            config.state_dir = PathBuf::from(dir);
        }
        if let Ok(pipe) = std::env::var("TPT_RUNTIME_PIPE") {
            if !pipe.is_empty() {
                config.pipe_name = pipe;
            }
        }
        if let Ok(tcp) = std::env::var("TPT_RUNTIME_TCP") {
            if !tcp.is_empty() {
                config.tcp = Some(tcp);
            }
        }
        config
    }

    /// `logs/` under the state directory.
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    /// `volumes/` under the state directory.
    pub fn volumes_dir(&self) -> PathBuf {
        self.state_dir.join("volumes")
    }

    /// `events.jsonl` under the state directory.
    pub fn events_file(&self) -> PathBuf {
        self.state_dir.join("events.jsonl")
    }

    /// `secrets.json` under the state directory.
    pub fn secrets_file(&self) -> PathBuf {
        self.state_dir.join("secrets.json")
    }

    /// `registry.jsonl` under the state directory: the workload registry
    /// snapshot used for restart reconciliation (SPEC Phase 9).
    pub fn registry_file(&self) -> PathBuf {
        self.state_dir.join("registry.jsonl")
    }

    /// Ensures the state directory layout exists.
    pub fn prepare_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.volumes_dir())?;
        Ok(())
    }
}

fn default_state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TPT_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("tpt").join("runtime");
    }
    // non-Windows fallback
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("tpt")
            .join("runtime");
    }
    PathBuf::from(".tpt-runtime")
}

fn pipe_name_from_env() -> Option<String> {
    std::env::var("TPT_RUNTIME_PIPE")
        .ok()
        .filter(|p| !p.is_empty())
}

fn tcp_from_env() -> Option<String> {
    std::env::var("TPT_RUNTIME_TCP")
        .ok()
        .filter(|p| !p.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pipe_is_local_named_pipe() {
        assert_eq!(DaemonConfig::default().pipe_name, DEFAULT_PIPE_NAME);
        assert!(DEFAULT_PIPE_NAME.starts_with(r"\\.\pipe\"));
    }

    #[test]
    fn state_layout_helpers() {
        let config = DaemonConfig {
            state_dir: std::env::temp_dir().join(format!("tpt-test-{}", std::process::id())),
            ..DaemonConfig::default()
        };
        config.prepare_dirs().unwrap();
        assert!(config.logs_dir().is_dir());
        assert!(config.volumes_dir().is_dir());
        std::fs::remove_dir_all(&config.state_dir).ok();
    }
}
