use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::{Transport, TransportError, TransportReceiver, TransportSender};

/// Newline-delimited JSON transport over a pair of async streams.
///
/// Used for both the host-facing side (relay stdin/stdout) and the downstream
/// side (child process stdin/stdout).
pub struct StdioTransport<R, W>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    reader: BufReader<R>,
    writer: W,
}

impl<R, W> StdioTransport<R, W>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
        }
    }

    /// Split into independent reader and writer halves.
    ///
    /// The writer half implements `TransportSender` and can be wrapped in
    /// `Arc<Mutex<>>` for the D-10 host-side handle seam.
    pub fn split(self) -> (TransportReader<R>, TransportWriter<W>) {
        (
            TransportReader {
                reader: self.reader,
            },
            TransportWriter {
                writer: self.writer,
            },
        )
    }
}

#[async_trait]
impl<R, W> Transport for StdioTransport<R, W>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        recv_line(&mut self.reader).await
    }

    async fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
        send_line(&mut self.writer, message).await
    }
}

/// Read half of a split StdioTransport.
pub struct TransportReader<R>
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    reader: BufReader<R>,
}

#[async_trait]
impl<R> TransportReceiver for TransportReader<R>
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        recv_line(&mut self.reader).await
    }
}

/// Write half of a split StdioTransport.
pub struct TransportWriter<W>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    writer: W,
}

#[async_trait]
impl<W> TransportSender for TransportWriter<W>
where
    W: tokio::io::AsyncWrite + Unpin + Send,
{
    async fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
        send_line(&mut self.writer, message).await
    }
}

// Shared implementation functions

async fn recv_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, TransportError> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| TransportError::Io {
                detail: e.to_string(),
            })?;
        if n == 0 {
            return Err(TransportError::Closed);
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.as_bytes().to_vec());
        }
    }
}

async fn send_line<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    message: &[u8],
) -> Result<(), TransportError> {
    writer
        .write_all(message)
        .await
        .map_err(|e| TransportError::Io {
            detail: e.to_string(),
        })?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| TransportError::Io {
            detail: e.to_string(),
        })?;
    writer.flush().await.map_err(|e| TransportError::Io {
        detail: e.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recv_reads_newline_delimited_messages() {
        let input = b"{\"jsonrpc\":\"2.0\"}\n{\"id\":1}\n";
        let reader = &input[..];
        let writer = Vec::new();
        let mut transport = StdioTransport::new(reader, writer);

        let msg1 = transport.recv().await.unwrap();
        assert_eq!(msg1, b"{\"jsonrpc\":\"2.0\"}");

        let msg2 = transport.recv().await.unwrap();
        assert_eq!(msg2, b"{\"id\":1}");
    }

    #[tokio::test]
    async fn recv_skips_blank_lines() {
        let input = b"\n\n{\"id\":1}\n\n";
        let reader = &input[..];
        let writer = Vec::new();
        let mut transport = StdioTransport::new(reader, writer);

        let msg = transport.recv().await.unwrap();
        assert_eq!(msg, b"{\"id\":1}");
    }

    #[tokio::test]
    async fn recv_returns_closed_on_eof() {
        let input = b"";
        let reader = &input[..];
        let writer = Vec::new();
        let mut transport = StdioTransport::new(reader, writer);

        let err = transport.recv().await.unwrap_err();
        assert!(matches!(err, TransportError::Closed));
    }

    #[tokio::test]
    async fn send_appends_newline_and_flushes() {
        let reader = &b""[..];
        let writer = Vec::new();
        let mut transport = StdioTransport::new(reader, writer);

        transport.send(b"{\"id\":1}").await.unwrap();

        let written = &transport.writer;
        assert_eq!(written, b"{\"id\":1}\n");
    }

    #[tokio::test]
    async fn round_trip() {
        // Write messages then read them back
        let mut buf = Vec::new();
        {
            let reader = &b""[..];
            let mut transport = StdioTransport::new(reader, &mut buf);
            transport.send(b"{\"msg\":1}").await.unwrap();
            transport.send(b"{\"msg\":2}").await.unwrap();
        }

        // Now read them back
        let reader = &buf[..];
        let writer = Vec::new();
        let mut transport = StdioTransport::new(reader, writer);

        let msg1 = transport.recv().await.unwrap();
        assert_eq!(msg1, b"{\"msg\":1}");

        let msg2 = transport.recv().await.unwrap();
        assert_eq!(msg2, b"{\"msg\":2}");
    }

    #[tokio::test]
    async fn split_reader_writer_work_independently() {
        let input = b"{\"id\":1}\n{\"id\":2}\n";
        let writer = Vec::new();
        let transport = StdioTransport::new(&input[..], writer);

        let (mut reader, mut writer) = transport.split();

        let msg1 = reader.recv().await.unwrap();
        assert_eq!(msg1, b"{\"id\":1}");

        writer.send(b"{\"out\":1}").await.unwrap();

        let msg2 = reader.recv().await.unwrap();
        assert_eq!(msg2, b"{\"id\":2}");
    }
}
