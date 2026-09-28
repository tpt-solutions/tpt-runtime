//! Newline-delimited JSON framing over async byte streams.
//!
//! Readers must be persistent [`AsyncBufRead`] wrappers (e.g. hold one
//! `BufReader` per connection); a fresh buffer per message would drop
//! any bytes read past the current line.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// Reads one newline-terminated JSON message.
///
/// Returns `Ok(None)` on clean EOF (peer closed after finishing its stream).
/// Pass a persistent buffered reader so bytes beyond the current line are
/// preserved for the next call.
pub async fn read_message<T: DeserializeOwned, R: AsyncBufRead + Unpin + ?Sized>(
    reader: &mut R,
) -> tpt_runtime_core::Result<Option<T>> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).await?;
    if bytes == 0 {
        return Ok(None);
    }
    serde_json::from_str::<T>(line.trim()).map(Some).map_err(|err| {
        tpt_runtime_core::error::RuntimeError::new(
            tpt_runtime_core::error::ErrorKind::Other,
            format!("malformed message: {err}"),
        )
    })
}

/// Writes one JSON message terminated by a newline and flushes.
pub async fn write_message<T: Serialize, W: AsyncWrite + Unpin + ?Sized>(
    writer: &mut W,
    message: &T,
) -> tpt_runtime_core::Result<()> {
    let mut bytes = serde_json::to_vec(message).map_err(|err| {
        tpt_runtime_core::error::RuntimeError::new(
            tpt_runtime_core::error::ErrorKind::Other,
            format!("cannot serialize message: {err}"),
        )
    })?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::DuplexStream;

    fn duplex() -> (DuplexStream, DuplexStream) {
        tokio::io::duplex(1024)
    }

    #[tokio::test]
    async fn messages_round_trip() {
        let (mut client, server) = duplex();
        write_message(&mut client, &serde_json::json!({"hello": "world"}))
            .await
            .unwrap();
        let mut server = tokio::io::BufReader::new(server);
        let received: serde_json::Value = read_message(&mut server).await.unwrap().unwrap();
        assert_eq!(received["hello"], "world");
    }

    #[tokio::test]
    async fn multiple_messages_in_order() {
        let (mut client, server) = duplex();
        for i in 0..5u32 {
            write_message(&mut client, &i).await.unwrap();
        }
        drop(client);
        let mut server = tokio::io::BufReader::new(server);
        for i in 0..5u32 {
            let value: u32 = read_message(&mut server).await.unwrap().unwrap();
            assert_eq!(value, i);
        }
        assert!(read_message::<u32, _>(&mut server).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn pipelined_messages_are_not_lost() {
        // The client writes everything before the server reads: buffered
        // bytes beyond the first line must survive the read call.
        let (client, server) = duplex();
        let writer = tokio::spawn(async move {
            let mut client = client;
            for i in 0..50u32 {
                write_message(&mut client, &i).await.unwrap();
            }
        });
        let mut server = tokio::io::BufReader::new(server);
        for i in 0..50u32 {
            let value: u32 = read_message(&mut server).await.unwrap().unwrap();
            assert_eq!(value, i);
        }
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_message_is_an_error() {
        let (mut client, server) = duplex();
        use tokio::io::AsyncWriteExt;
        client.write_all(b"not json\n").await.unwrap();
        drop(client);
        let mut server = tokio::io::BufReader::new(server);
        let err = read_message::<serde_json::Value, _>(&mut server)
            .await
            .unwrap_err();
        assert!(err.message.contains("malformed"));
    }
}
