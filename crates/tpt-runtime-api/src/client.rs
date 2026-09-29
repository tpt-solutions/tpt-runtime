//! API client used by the CLI (SPEC §30: CLI is an API client).

use std::time::Duration;
use tokio::io::BufReader;
use tpt_runtime_config::DaemonConfig;
use tpt_runtime_core::error::{ErrorKind, Result, RuntimeError};
use tpt_runtime_core::event::RuntimeEvent;
use tpt_runtime_ipc::{read_message, write_message, Request, Response};

/// One open connection to the runtime daemon.
pub struct ApiClient {
    reader: BufReader<ReadHalf>,
    writer: WriteHalf,
    next_id: u64,
}

#[cfg(windows)]
type ReadHalf = tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeClient>;
#[cfg(windows)]
type WriteHalf = tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>;
#[cfg(not(windows))]
type ReadHalf = tokio::io::ReadHalf<tokio::net::TcpStream>;
#[cfg(not(windows))]
type WriteHalf = tokio::io::WriteHalf<tokio::net::TcpStream>;

impl ApiClient {
    /// Connects to the daemon's pipe with a short retry window (the daemon
    /// creates pipe instances on demand).
    pub async fn connect(config: &DaemonConfig) -> Result<Self> {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            #[cfg(windows)]
            let attempt =
                tokio::net::windows::named_pipe::ClientOptions::new().open(&config.pipe_name);
            #[cfg(not(windows))]
            let attempt = {
                let port: u16 = config
                    .pipe_name
                    .rsplit(':')
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(7900);
                tokio::net::TcpStream::connect(("127.0.0.1", port)).await
            };
            match attempt {
                Ok(stream) => {
                    let (read_half, write_half) = tokio::io::split(stream);
                    return Ok(Self {
                        reader: BufReader::new(read_half),
                        writer: write_half,
                        next_id: 1,
                    });
                }
                Err(err) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(RuntimeError::new(
                            ErrorKind::NotFound,
                            format!(
                                "runtime daemon is not reachable on '{}': {err} (start it with 'tpt daemon start')",
                                config.pipe_name
                            ),
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Performs one request/response exchange.
    pub async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            id,
            method: method.to_owned(),
            params,
        };
        write_message(&mut self.writer, &request).await?;
        // read responses, skipping any interleaved event lines (this
        // connection did not subscribe)
        loop {
            let response: Response = read_message(&mut self.reader).await?.ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::System,
                    "daemon closed the connection before responding",
                )
            })?;
            if response.id == id {
                return match response.error {
                    Some(err) => Err(RuntimeError::new(
                        err.kind.parse().unwrap_or(ErrorKind::Other),
                        err.message,
                    )
                    .with_operation(method)),
                    None => Ok(response.result.unwrap_or(serde_json::Value::Null)),
                };
            }
        }
    }

    /// Receives one event line (only meaningful on a subscribed connection).
    pub async fn recv_event(&mut self) -> Result<RuntimeEvent> {
        let event: RuntimeEvent = read_message(&mut self.reader)
            .await?
            .ok_or_else(|| RuntimeError::new(ErrorKind::System, "event stream closed by daemon"))?;
        Ok(event)
    }

    /// Subscribes this connection to the live event stream.
    pub async fn subscribe(&mut self) -> Result<()> {
        self.call("events.subscribe", serde_json::Value::Null)
            .await
            .map(|_| ())
    }
}
