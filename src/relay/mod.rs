pub mod carveouts;

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::transport::{Transport, TransportError, TransportReceiver, TransportSender};
use carveouts::apply_sampling_carveout;

/// Run the MCP proxy loop: forward messages between host and downstream,
/// applying the sampling carve-out on the first host→downstream message.
///
/// The host-facing side is split: a `TransportReceiver` for reading and an
/// `Arc<Mutex<dyn TransportSender>>` for writing (the D-10 seam for post-MVP
/// sampling support). The downstream side uses the unified `Transport` trait.
///
/// Returns `Ok(())` when either side closes cleanly (EOF), or `Err` on
/// transport I/O errors.
pub async fn proxy_loop(
    host_reader: &mut dyn TransportReceiver,
    host_writer: &Arc<Mutex<dyn TransportSender>>,
    downstream: &mut dyn Transport,
    verbose: bool,
) -> Result<(), TransportError> {
    let mut first_message = true;

    loop {
        // Biased toward host: process all host messages before checking
        // downstream. This ensures host EOF triggers drain_downstream only
        // after all host messages are forwarded. In production, real async
        // I/O has independent timing so the bias is harmless.
        tokio::select! {
            biased;
            result = host_reader.recv() => {
                match result {
                    Ok(msg) => {
                        let forwarded = if first_message {
                            first_message = false;
                            apply_sampling_carveout(&msg)
                        } else {
                            msg
                        };
                        downstream.send(&forwarded).await?;
                    }
                    Err(TransportError::Closed) => {
                        if verbose {
                            eprintln!("[mcp-vault-wrap] Host closed connection, draining downstream");
                        }
                        // Host closed — drain any remaining downstream messages
                        // before exiting. The downstream will close when the
                        // child process exits (stdout EOF).
                        return drain_downstream(host_writer, downstream, verbose).await;
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("[mcp-vault-wrap] Host transport error: {e}");
                        }
                        return Err(e);
                    }
                }
            }
            result = downstream.recv() => {
                match result {
                    Ok(msg) => {
                        host_writer.lock().await.send(&msg).await?;
                    }
                    Err(TransportError::Closed) => {
                        if verbose {
                            eprintln!("[mcp-vault-wrap] Downstream server closed connection");
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("[mcp-vault-wrap] Downstream transport error: {e}");
                        }
                        return Err(e);
                    }
                }
            }
        }
    }
}

/// After the host closes, forward any remaining downstream messages to the
/// host writer until the downstream also closes.
async fn drain_downstream(
    host_writer: &Arc<Mutex<dyn TransportSender>>,
    downstream: &mut dyn Transport,
    verbose: bool,
) -> Result<(), TransportError> {
    loop {
        match downstream.recv().await {
            Ok(msg) => {
                host_writer.lock().await.send(&msg).await?;
            }
            Err(TransportError::Closed) => {
                if verbose {
                    eprintln!("[mcp-vault-wrap] Downstream closed after drain");
                }
                return Ok(());
            }
            Err(e) => {
                if verbose {
                    eprintln!("[mcp-vault-wrap] Downstream error during drain: {e}");
                }
                return Err(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::VecDeque;

    /// Mock reader for the host side.
    struct MockReader {
        incoming: VecDeque<Result<Vec<u8>, TransportError>>,
    }

    impl MockReader {
        fn new(incoming: Vec<Result<Vec<u8>, TransportError>>) -> Self {
            Self {
                incoming: VecDeque::from(incoming),
            }
        }
    }

    #[async_trait]
    impl TransportReceiver for MockReader {
        async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
            match self.incoming.pop_front() {
                Some(result) => result,
                None => std::future::pending().await,
            }
        }
    }

    /// Mock sender for the host side.
    struct MockSender {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    #[async_trait]
    impl TransportSender for MockSender {
        async fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
            self.sent.lock().await.push(message.to_vec());
            Ok(())
        }
    }

    /// Mock transport for the downstream side (unified recv + send).
    struct MockTransport {
        incoming: VecDeque<Result<Vec<u8>, TransportError>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl MockTransport {
        fn new(
            incoming: Vec<Result<Vec<u8>, TransportError>>,
            sent: Arc<Mutex<Vec<Vec<u8>>>>,
        ) -> Self {
            Self {
                incoming: VecDeque::from(incoming),
                sent,
            }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
            match self.incoming.pop_front() {
                Some(result) => result,
                None => std::future::pending().await,
            }
        }

        async fn send(&mut self, message: &[u8]) -> Result<(), TransportError> {
            self.sent.lock().await.push(message.to_vec());
            Ok(())
        }
    }

    fn make_host(
        incoming: Vec<Result<Vec<u8>, TransportError>>,
    ) -> (
        MockReader,
        Arc<Mutex<dyn TransportSender>>,
        Arc<Mutex<Vec<Vec<u8>>>>,
    ) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let reader = MockReader::new(incoming);
        let writer: Arc<Mutex<dyn TransportSender>> =
            Arc::new(Mutex::new(MockSender { sent: sent.clone() }));
        (reader, writer, sent)
    }

    #[tokio::test]
    async fn passthrough_both_directions() {
        let (mut host_reader, host_writer, host_sent) = make_host(vec![
            Ok(b"{\"method\":\"tools/list\",\"id\":1}".to_vec()),
            Err(TransportError::Closed),
        ]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream = MockTransport::new(
            vec![
                Ok(b"{\"result\":[],\"id\":1}".to_vec()),
                Err(TransportError::Closed),
            ],
            downstream_sent.clone(),
        );

        proxy_loop(&mut host_reader, &host_writer, &mut downstream, false)
            .await
            .unwrap();

        let ds = downstream_sent.lock().await;
        assert_eq!(ds.len(), 1);

        let hs = host_sent.lock().await;
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0], b"{\"result\":[],\"id\":1}");
    }

    #[tokio::test]
    async fn sampling_carveout_on_first_message_only() {
        let init_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {
                    "sampling": {},
                    "roots": {}
                }
            }
        });
        let second_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });

        let (mut host_reader, host_writer, _host_sent) = make_host(vec![
            Ok(serde_json::to_vec(&init_msg).unwrap()),
            Ok(serde_json::to_vec(&second_msg).unwrap()),
            Err(TransportError::Closed),
        ]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream =
            MockTransport::new(vec![Err(TransportError::Closed)], downstream_sent.clone());

        proxy_loop(&mut host_reader, &host_writer, &mut downstream, false)
            .await
            .unwrap();

        let ds = downstream_sent.lock().await;
        assert_eq!(ds.len(), 2);

        // First message: sampling should be stripped
        let first: serde_json::Value = serde_json::from_slice(&ds[0]).unwrap();
        assert!(first["params"]["capabilities"]["sampling"].is_null());
        assert!(first["params"]["capabilities"]["roots"].is_object());

        // Second message: forwarded as-is
        assert_eq!(ds[1], serde_json::to_vec(&second_msg).unwrap());
    }

    #[tokio::test]
    async fn unknown_jsonrpc_forwarded_as_is() {
        let unknown = b"{\"jsonrpc\":\"2.0\",\"method\":\"custom/weird\",\"id\":99}";
        let (mut host_reader, host_writer, _host_sent) =
            make_host(vec![Ok(unknown.to_vec()), Err(TransportError::Closed)]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream =
            MockTransport::new(vec![Err(TransportError::Closed)], downstream_sent.clone());

        proxy_loop(&mut host_reader, &host_writer, &mut downstream, false)
            .await
            .unwrap();

        let ds = downstream_sent.lock().await;
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0], unknown.to_vec());
    }

    #[tokio::test]
    async fn malformed_message_forwarded_as_is() {
        let garbage = b"this is not json";
        let (mut host_reader, host_writer, _host_sent) =
            make_host(vec![Ok(garbage.to_vec()), Err(TransportError::Closed)]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream =
            MockTransport::new(vec![Err(TransportError::Closed)], downstream_sent.clone());

        proxy_loop(&mut host_reader, &host_writer, &mut downstream, false)
            .await
            .unwrap();

        let ds = downstream_sent.lock().await;
        assert_eq!(ds[0], garbage.to_vec());
    }

    #[tokio::test]
    async fn downstream_close_terminates_loop() {
        let (mut host_reader, host_writer, _host_sent) = make_host(vec![]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream =
            MockTransport::new(vec![Err(TransportError::Closed)], downstream_sent.clone());

        let result = proxy_loop(&mut host_reader, &host_writer, &mut downstream, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn transport_io_error_returns_err() {
        let (mut host_reader, host_writer, _host_sent) = make_host(vec![Err(TransportError::Io {
            detail: "broken pipe".to_string(),
        })]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream = MockTransport::new(vec![], downstream_sent.clone());

        let result = proxy_loop(&mut host_reader, &host_writer, &mut downstream, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn message_ordering_preserved() {
        let (mut host_reader, host_writer, _host_sent) = make_host(vec![
            Ok(b"{\"id\":1}".to_vec()),
            Ok(b"{\"id\":2}".to_vec()),
            Ok(b"{\"id\":3}".to_vec()),
            Err(TransportError::Closed),
        ]);
        let downstream_sent = Arc::new(Mutex::new(Vec::new()));
        let mut downstream =
            MockTransport::new(vec![Err(TransportError::Closed)], downstream_sent.clone());

        proxy_loop(&mut host_reader, &host_writer, &mut downstream, false)
            .await
            .unwrap();

        let ds = downstream_sent.lock().await;
        assert_eq!(ds.len(), 3);
        assert_eq!(ds[0], b"{\"id\":1}");
        assert_eq!(ds[1], b"{\"id\":2}");
        assert_eq!(ds[2], b"{\"id\":3}");
    }
}
