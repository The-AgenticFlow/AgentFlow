// crates/coder-client/src/mock_chat_server.rs
//! Mock WebSocket chat server for testing the ChatStream implementation.
//!
//! Provides a lightweight server that mimics Coder's `/api/v2/chats/{id}/stream`
//! WebSocket endpoint, emitting predetermined events for testing.

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

/// A mock chat server that emits predefined events over WebSocket.
pub struct MockChatServer {
    addr: SocketAddr,
    server_handle: Option<tokio::task::JoinHandle<()>>,
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl MockChatServer {
    /// Create a new mock server bound to a random available port.
    pub async fn new() -> anyhow::Result<Self> {
        // Bind to random port on localhost
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let events = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(Mutex::new(None));

        let server_handle = tokio::spawn(Self::run_server(
            listener,
            Arc::clone(&events),
            Arc::clone(&shutdown),
        ));

        Ok(Self {
            addr,
            server_handle: Some(server_handle),
            events,
            shutdown,
        })
    }

    /// Add events that the server will emit to connected clients.
    pub async fn add_events(&self, events: Vec<serde_json::Value>) {
        let mut guard = self.events.lock().await;
        guard.extend(events);
    }

    /// Set the complete event sequence (replaces any existing events).
    pub async fn set_events(&self, events: Vec<serde_json::Value>) {
        let mut guard = self.events.lock().await;
        *guard = events;
    }

    /// Get the base WebSocket URL (ws://127.0.0.1:{port}).
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Get the full v2 stream endpoint for a chat session.
    ///
    /// Mimics Coder's `/api/v2/chats/{id}/stream` WebSocket route so tests
    /// exercise the same path the client connects to.
    pub fn v2_stream_url(&self, chat_id: &str) -> String {
        format!("{}/api/v2/chats/{}/stream", self.ws_url(), chat_id)
    }

    /// Get just the host:port part (e.g., "127.0.0.1:8080").
    pub fn addr_str(&self) -> String {
        self.addr.to_string()
    }

    /// Shutdown the server gracefully.
    pub async fn shutdown(self) {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server_handle {
            let _ = handle.await;
        }
    }

    async fn run_server(
        listener: TcpListener,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
        shutdown: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    ) {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut guard = shutdown.lock().await;
            *guard = Some(shutdown_tx);
        }

        loop {
            tokio::select! {
                // Wait for incoming connections
                Ok((stream, peer_addr)) = listener.accept() => {
                    info!(peer = %peer_addr, "Mock chat server: new WS connection");
                    let events_clone = Arc::clone(&events);
                    tokio::spawn(async move {
                        // Echo the requested subprotocol so the client's
                        // `Sec-WebSocket-Protocol: chat-stream-v1` handshake
                        // succeeds (the real Coder server negotiates it too),
                        // and reject connections to any non-stream route so the
                        // mock independently verifies the client's target URL.
                        if let Ok(ws_stream) = accept_v2_stream_handshake(stream).await
                        {
                            let (mut _ws_sink, _ws_stream) = ws_stream.split();

                            // Emit events to the client sequentially
                            let events_snapshot = {
                                let guard = events_clone.lock().await;
                                guard.clone()
                            };

                            for event in events_snapshot {
                                let msg = Message::Text(serde_json::to_string(&event).unwrap_or_default().into());
                                if let Err(e) = _ws_sink.send(msg).await {
                                    warn!(peer = %peer_addr, error = %e, "Failed to send event");
                                    break;
                                }
                                // Small delay between events to simulate real streaming
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            }

                            // Always end with a finished event
                            let finished = json!({
                                "type": "finished",
                                "final_output": "Mock server: chat completed"
                            });
                            let _ = _ws_sink.send(Message::Text(serde_json::to_string(&finished).unwrap().into())).await;

                            info!(peer = %peer_addr, "Mock chat server: finished emitting events");
                        }
                    });
                }
                // Shutdown signal
                _ = &mut shutdown_rx => {
                    info!("Mock chat server shutting down");
                    break;
                }
            }
        }
    }
}

/// Accept a WebSocket connection for the mock chat server, but only when the
/// client is connecting to the Coder v2 chat stream route
/// `/api/v2/chats/{chat_id}/stream`. Any other URI is rejected so the mock
/// independently verifies the client's target endpoint rather than accepting
/// every path.
///
/// The handshake callback's `Err` variant is inherently large (tungstenite's
/// `Error` type), so `result_large_err` is suppressed here.
#[allow(clippy::result_large_err)]
async fn accept_v2_stream_handshake(
    stream: TcpStream,
) -> Result<
    tokio_tungstenite::WebSocketStream<TcpStream>,
    tokio_tungstenite::tungstenite::error::Error,
> {
    tokio_tungstenite::accept_hdr_async(
        stream,
        |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
         mut resp: tokio_tungstenite::tungstenite::handshake::server::Response| {
            let path = req.uri().path();
            let is_v2_stream = path.starts_with("/api/v2/chats/") && path.ends_with("/stream");
            if !is_v2_stream {
                warn!(
                    uri = %req.uri(),
                    "Mock chat server: rejecting handshake for non-v2-stream path"
                );
                let err_resp: tokio_tungstenite::tungstenite::handshake::server::ErrorResponse =
                    tokio_tungstenite::tungstenite::http::Response::builder()
                        .status(tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND)
                        .body(None)
                        .expect("valid static error response");
                return Err(err_resp);
            }

            // Echo the negotiated subprotocol so the client's handshake succeeds.
            resp.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                "chat-stream-v1".parse().expect("valid static header value"),
            );
            Ok(resp)
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_server_creation() {
        let server = MockChatServer::new()
            .await
            .expect("Failed to create mock server");
        let url = server.ws_url();
        assert!(url.starts_with("ws://127.0.0.1:"));
        // The v2 stream endpoint helper must point at the Coder v2 route.
        assert_eq!(
            server.v2_stream_url("chat-123"),
            format!("{}/api/v2/chats/chat-123/stream", url)
        );
        // Server is dropped here, which will terminate the task
    }

    #[tokio::test]
    async fn test_mock_server_add_events() {
        let server = MockChatServer::new().await.unwrap();
        server
            .add_events(vec![
                json!({"type": "status_update", "status": "pending"}),
                json!({"type": "text", "content": "Hello from mock"}),
            ])
            .await;

        // Verify events were stored
        let events = server.events.lock().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "status_update");
    }
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;

    /// Create a mock server pre-configured with a typical chat event sequence.
    pub async fn create_typical_mock() -> MockChatServer {
        let server = MockChatServer::new()
            .await
            .expect("Failed to create mock server");

        server
            .set_events(vec![
                json!({"type": "status_update", "status": "pending"}),
                json!({"type": "status_update", "status": "running"}),
                json!({"type": "text", "content": "Hello from mock chat server"}),
                json!({"type": "finished", "final_output": "Chat completed successfully"}),
            ])
            .await;

        server
    }
}
