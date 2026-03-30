pub mod stdio;

use std::fmt;

use async_trait::async_trait;

/// Async transport trait for MCP message passing.
///
/// Each `recv()` returns one complete JSON-RPC message as raw bytes.
/// Each `send()` writes one complete message. The transport owns framing.
#[async_trait]
pub trait Transport: Send {
    async fn recv(&mut self) -> Result<Vec<u8>, TransportError>;
    async fn send(&mut self, message: &[u8]) -> Result<(), TransportError>;
}

/// Read half of a transport — can receive messages independently.
#[async_trait]
pub trait TransportReceiver: Send {
    async fn recv(&mut self) -> Result<Vec<u8>, TransportError>;
}

/// Write half of a transport — can send messages independently.
///
/// Extracted so the host-facing send side can be wrapped in Arc<Mutex<>> as
/// the D-10 sampling expansion seam.
#[async_trait]
pub trait TransportSender: Send {
    async fn send(&mut self, message: &[u8]) -> Result<(), TransportError>;
}

#[derive(Debug)]
pub enum TransportError {
    /// The transport stream has ended (EOF).
    Closed,
    /// An I/O or framing error occurred.
    Io { detail: String },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Closed => write!(f, "Transport closed"),
            TransportError::Io { detail } => write!(f, "Transport I/O error: {detail}"),
        }
    }
}

impl std::error::Error for TransportError {}
