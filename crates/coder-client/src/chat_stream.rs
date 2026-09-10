// crates/coder-client/src/chat_stream.rs
//! WebSocket-based streaming for Coder Chat sessions.
//!
//! Provides the `ChatStream` type that wraps a WebSocket connection
//! to receive real-time streaming responses from the Chats API.

use anyhow::{Context, Result};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, warn};

/// A streamed chat event from the WebSocket connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// Text chunk in the response.
    Text { content: String },
    /// The chat session has finished (terminal state).
    Finished { final_output: Option<String> },
    /// Status update during the chat lifecycle.
    StatusUpdate { status: String },
    /// Error from the chat session.
    Error { message: String },
    /// Heartbeat / keepalive from server.
    Ping,
    /// Unknown or unexpected event type.
    Unknown { raw: serde_json::Value },
}

impl ChatEvent {
    /// Parse a ChatEvent from raw JSON received over WebSocket.
    pub fn from_json(value: serde_json::Value) -> Self {
        // Try each variant in priority order
        if let Some(text) = value.get("content").and_then(|v| v.as_str()) {
            if value.get("type").and_then(|v| v.as_str()) == Some("text") {
                return ChatEvent::Text {
                    content: text.to_string(),
                };
            }
        }

        if let Some(t) = value.get("type").and_then(|v| v.as_str()) {
            match t {
                "finished" => ChatEvent::Finished {
                    final_output: value
                        .get("final_output")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                },
                "status_update" => ChatEvent::StatusUpdate {
                    status: value
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                },
                "error" => ChatEvent::Error {
                    message: value
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error")
                        .to_string(),
                },
                "ping" => ChatEvent::Ping,
                other => {
                    debug!(event_type = other, "Received unknown chat event type");
                    ChatEvent::Unknown { raw: value }
                }
            }
        } else {
            // Fallback: check for content field as text
            if let Some(text) = value.get("content").and_then(|v| v.as_str()) {
                return ChatEvent::Text {
                    content: text.to_string(),
                };
            }
            ChatEvent::Unknown { raw: value }
        }
    }
}

/// A WebSocket stream for receiving real-time chat events.
///
/// Connects to the Coder Chats API WebSocket endpoint and yields `ChatEvent`
/// instances as they arrive. The stream ends when the chat session completes.
pub struct ChatStream {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    chat_id: String,
    finished: bool,
}

impl ChatStream {
    /// Connect to a chat session WebSocket and create a streaming reader.
    ///
    /// # Arguments
    /// * `coder_url` - Base URL of the Coder server (e.g., "http://localhost:3000")
    /// * `token` - Authentication token for the WebSocket connection
    /// * `chat_id` - ID of the chat session to stream
    pub async fn connect(coder_url: &str, token: &str, chat_id: &str) -> Result<Self> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let ws_url = Self::stream_url(coder_url, chat_id);

        debug!(url = %ws_url, chat_id, "Connecting to chat WebSocket stream");

        let mut request = ws_url
            .clone()
            .into_client_request()
            .context("Failed to build WebSocket request")?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", token)
                .parse()
                .context("Invalid token")?,
        );
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            "chat-stream-v1"
                .parse()
                .context("Invalid protocol header")?,
        );

        let (ws, _resp) = connect_async(request)
            .await
            .with_context(|| format!("Failed to connect to WebSocket endpoint: {}", ws_url))?;

        Ok(Self {
            ws,
            chat_id: chat_id.to_string(),
            finished: false,
        })
    }

    /// Build the v2 WebSocket stream URL for a chat session.
    ///
    /// Points at Coder's `/api/v2/chats/{chat_id}/stream` endpoint. An
    /// `http(s)://` base URL is translated to `ws(s)://`; a `ws(s)://` base
    /// URL is used as-is.
    fn stream_url(coder_url: &str, chat_id: &str) -> String {
        let path = format!("/api/v2/chats/{}/stream", chat_id);
        if coder_url.starts_with("ws") {
            format!("{}{}", coder_url.trim_end_matches('/'), path)
        } else {
            coder_url
                .replace("http://", "ws://")
                .replace("https://", "wss://")
                .trim_end_matches('/')
                .to_string()
                + &path
        }
    }

    /// The chat ID this stream is reading from.
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    /// Whether the chat session has reached a terminal state.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Process a single WebSocket message into a `ChatEvent`.
    fn process_message(msg: Message) -> Option<ChatEvent> {
        match msg {
            Message::Text(text) => {
                let text_str = text.to_string();
                match serde_json::from_str::<serde_json::Value>(&text_str) {
                    Ok(json) => Some(ChatEvent::from_json(json)),
                    Err(_) => {
                        warn!(raw_text = %text_str, "Failed to parse WebSocket message as JSON");
                        Some(ChatEvent::Error {
                            message: "Invalid JSON in WebSocket message".to_string(),
                        })
                    }
                }
            }
            Message::Binary(data) => match serde_json::from_slice::<serde_json::Value>(&data) {
                Ok(json) => Some(ChatEvent::from_json(json)),
                Err(_) => {
                    warn!("Received binary WebSocket message, skipping");
                    None
                }
            },
            Message::Close(_) => {
                debug!("WebSocket connection closed");
                None
            }
            Message::Ping(payload) => {
                debug!(?payload, "Received WebSocket ping");
                Some(ChatEvent::Ping)
            }
            Message::Pong(_payload) => Some(ChatEvent::Ping),
            Message::Frame(_) => None,
        }
    }
}

impl Stream for ChatStream {
    type Item = Result<ChatEvent, tokio_tungstenite::tungstenite::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        match Pin::new(&mut self.ws).poll_next(cx) {
            Poll::Ready(Some(Ok(msg))) => match ChatStream::process_message(msg) {
                Some(ChatEvent::Finished { .. }) => {
                    self.finished = true;
                    Poll::Ready(Some(Ok(ChatEvent::Finished {
                        final_output: Some("Chat session completed".into()),
                    })))
                }
                Some(event) => Poll::Ready(Some(Ok(event))),
                None => Poll::Pending,
            },
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                self.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_event_parsing_text() {
        let json = serde_json::json!({
            "type": "text",
            "content": "Hello world"
        });
        match ChatEvent::from_json(json) {
            ChatEvent::Text { content } => assert_eq!(content, "Hello world"),
            other => panic!("Expected Text, got: {:?}", other),
        }
    }

    #[test]
    fn test_chat_event_parsing_finished() {
        let json = serde_json::json!({
            "type": "finished",
            "final_output": "Done"
        });
        match ChatEvent::from_json(json) {
            ChatEvent::Finished { final_output } => {
                assert_eq!(final_output, Some("Done".to_string()))
            }
            other => panic!("Expected Finished, got: {:?}", other),
        }
    }

    #[test]
    fn test_chat_event_parsing_error() {
        let json = serde_json::json!({
            "type": "error",
            "message": "Something went wrong"
        });
        match ChatEvent::from_json(json) {
            ChatEvent::Error { message } => assert_eq!(message, "Something went wrong"),
            other => panic!("Expected Error, got: {:?}", other),
        }
    }

    #[test]
    fn test_chat_event_parsing_status_update() {
        let json = serde_json::json!({
            "type": "status_update",
            "status": "running"
        });
        match ChatEvent::from_json(json) {
            ChatEvent::StatusUpdate { status } => assert_eq!(status, "running"),
            other => panic!("Expected StatusUpdate, got: {:?}", other),
        }
    }

    #[test]
    fn stream_url_uses_v2_path_for_ws_prefixed_base() {
        let url = ChatStream::stream_url("ws://127.0.0.1:4321", "chat-123");
        assert_eq!(url, "ws://127.0.0.1:4321/api/v2/chats/chat-123/stream");
    }

    #[test]
    fn stream_url_translates_http_to_ws() {
        let url = ChatStream::stream_url("http://localhost:3000", "chat-123");
        assert_eq!(url, "ws://localhost:3000/api/v2/chats/chat-123/stream");
    }

    #[test]
    fn stream_url_translates_https_to_wss() {
        let url = ChatStream::stream_url("https://coder.example.com", "chat-9");
        assert_eq!(url, "wss://coder.example.com/api/v2/chats/chat-9/stream");
    }

    #[test]
    fn stream_url_handles_trailing_slash() {
        let url = ChatStream::stream_url("http://localhost:3000/", "chat-1");
        assert_eq!(url, "ws://localhost:3000/api/v2/chats/chat-1/stream");
    }

    #[tokio::test]
    async fn connect_reaches_v2_stream_endpoint_on_mock_server() {
        use futures::StreamExt;

        let server = crate::mock_chat_server::test_helpers::create_typical_mock().await;
        let url = server.ws_url();
        let mut stream = ChatStream::connect(&url, "test-token", "chat-123")
            .await
            .expect("connect should succeed against mock server");

        assert_eq!(stream.chat_id(), "chat-123");

        let events: Vec<_> = stream.by_ref().collect().await;
        assert!(!events.is_empty(), "expected events from mock stream");
        for event in &events {
            let event = event.as_ref().expect("stream item should be Ok");
            assert!(
                matches!(
                    event,
                    ChatEvent::StatusUpdate { .. }
                        | ChatEvent::Text { .. }
                        | ChatEvent::Finished { .. }
                ),
                "unexpected event {:?}",
                event
            );
        }
        server.shutdown().await;
    }
}
