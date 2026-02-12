//! WebSocket transport layer for connection lifecycle management
//!
//! This module provides a low-level transport abstraction that handles:
//! - Connection establishment
//! - Message sending/receiving with automatic ping/pong handling
//! - Reconnection with exponential backoff
//! - Connection statistics

use crate::errors::{PolyfillError, Result, StreamErrorKind};
use crate::stream::{ReconnectConfig, StreamStats};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, error, info, warn};

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Raw message received from WebSocket
#[derive(Debug, Clone)]
pub enum RawMessage {
    /// Text message content
    Text(String),
    /// Binary message content
    Binary(Vec<u8>),
}

/// Low-level WebSocket transport handling connection lifecycle
///
/// Encapsulates connection management, message framing, ping/pong handling,
/// and reconnection logic. Exchange-specific streams wrap this transport
/// and implement their own message parsing and subscription logic.
#[derive(Debug)]
pub struct WsTransport {
    /// Active WebSocket connection
    connection: Option<WsStream>,
    /// WebSocket URL
    url: String,
    /// Connection statistics
    stats: StreamStats,
    /// Reconnection configuration
    reconnect_config: ReconnectConfig,
    /// Last activity time for staleness detection
    last_activity: Option<std::time::Instant>,
}

impl WsTransport {
    /// Create a new transport for the given URL
    pub fn new(url: &str) -> Self {
        Self {
            connection: None,
            url: url.to_string(),
            stats: StreamStats {
                messages_received: 0,
                messages_sent: 0,
                errors: 0,
                last_message_time: None,
                connection_uptime: std::time::Duration::ZERO,
                reconnect_count: 0,
            },
            reconnect_config: ReconnectConfig::default(),
            last_activity: None,
        }
    }

    /// Set custom reconnection configuration
    pub fn with_reconnect_config(mut self, config: ReconnectConfig) -> Self {
        self.reconnect_config = config;
        self
    }

    /// Get the WebSocket URL
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Connect to the WebSocket server with default timeout from config
    pub async fn connect(&mut self) -> Result<()> {
        self.connect_with_timeout(self.reconnect_config.connect_timeout)
            .await
    }

    /// Connect to the WebSocket server with a custom timeout
    pub async fn connect_with_timeout(&mut self, timeout: std::time::Duration) -> Result<()> {
        info!(
            "Connecting to WebSocket at {} (timeout: {:?})",
            self.url, timeout
        );

        let connect_future = tokio_tungstenite::connect_async(&self.url);

        let (ws_stream, _) = tokio::time::timeout(timeout, connect_future)
            .await
            .map_err(|_| {
                PolyfillError::stream(
                    format!("WebSocket connection timed out after {:?}", timeout),
                    StreamErrorKind::ConnectionFailed,
                )
            })?
            .map_err(|e| {
                PolyfillError::stream(
                    format!("WebSocket connection failed: {}", e),
                    StreamErrorKind::ConnectionFailed,
                )
            })?;

        self.connection = Some(ws_stream);
        self.last_activity = Some(std::time::Instant::now());
        info!("Connected to WebSocket");
        Ok(())
    }

    /// Disconnect from the WebSocket server
    pub fn disconnect(&mut self) {
        self.connection = None;
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Send a serializable message
    pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<()> {
        let text = serde_json::to_string(message).map_err(|e| {
            PolyfillError::parse(format!("Failed to serialize message: {}", e), None)
        })?;
        self.send_text(text).await
    }

    /// Send a raw text message
    pub async fn send_text(&mut self, text: String) -> Result<()> {
        if let Some(connection) = &mut self.connection {
            let ws_message = WsMessage::Text(text);
            connection.send(ws_message).await.map_err(|e| {
                PolyfillError::stream(
                    format!("Failed to send message: {}", e),
                    StreamErrorKind::MessageCorrupted,
                )
            })?;
            self.stats.messages_sent += 1;
            Ok(())
        } else {
            Err(PolyfillError::stream(
                "Not connected",
                StreamErrorKind::ConnectionFailed,
            ))
        }
    }

    /// Send a ping message
    pub async fn ping(&mut self) -> Result<()> {
        if let Some(connection) = &mut self.connection {
            connection
                .send(WsMessage::Ping(vec![]))
                .await
                .map_err(|e| {
                    PolyfillError::stream(
                        format!("Failed to send ping: {}", e),
                        StreamErrorKind::MessageCorrupted,
                    )
                })?;
            Ok(())
        } else {
            Err(PolyfillError::stream(
                "Not connected",
                StreamErrorKind::ConnectionFailed,
            ))
        }
    }

    /// Receive the next message, handling ping/pong automatically
    ///
    /// Returns:
    /// - `Some(Ok(RawMessage))` for text/binary messages
    /// - `Some(Err(...))` on error
    /// - `None` on clean close or no connection
    pub async fn recv(&mut self) -> Option<Result<RawMessage>> {
        let connection = self.connection.as_mut()?;

        loop {
            match connection.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    if text.is_empty() {
                        continue;
                    }
                    self.stats.messages_received += 1;
                    self.stats.last_message_time = Some(Utc::now());
                    self.last_activity = Some(std::time::Instant::now());
                    return Some(Ok(RawMessage::Text(text)));
                },
                Some(Ok(WsMessage::Binary(data))) => {
                    self.stats.messages_received += 1;
                    self.stats.last_message_time = Some(Utc::now());
                    self.last_activity = Some(std::time::Instant::now());
                    return Some(Ok(RawMessage::Binary(data)));
                },
                Some(Ok(WsMessage::Ping(data))) => {
                    debug!("Received ping, sending pong");
                    self.last_activity = Some(std::time::Instant::now());
                    if let Err(e) = connection.send(WsMessage::Pong(data)).await {
                        error!("Failed to send pong: {}", e);
                    }
                    continue;
                },
                Some(Ok(WsMessage::Pong(_))) => {
                    debug!("Received pong");
                    self.last_activity = Some(std::time::Instant::now());
                    continue;
                },
                Some(Ok(WsMessage::Close(_))) => {
                    info!("WebSocket connection closed by server");
                    self.connection = None;
                    return None;
                },
                Some(Ok(WsMessage::Frame(_))) => {
                    continue;
                },
                Some(Err(e)) => {
                    self.stats.errors += 1;
                    return Some(Err(e.into()));
                },
                None => {
                    self.connection = None;
                    return None;
                },
            }
        }
    }

    /// Reconnect with exponential backoff
    ///
    /// Streams should call this when they detect connection issues,
    /// then resubscribe to their topics after successful reconnection.
    pub async fn reconnect(&mut self) -> Result<()> {
        let mut delay = self.reconnect_config.base_delay;
        let mut retries = 0;

        self.connection = None;

        while retries < self.reconnect_config.max_retries {
            warn!("Attempting to reconnect (attempt {})", retries + 1);

            match self.connect().await {
                Ok(()) => {
                    info!("Successfully reconnected");
                    self.stats.reconnect_count += 1;
                    return Ok(());
                },
                Err(e) => {
                    error!("Reconnection attempt {} failed: {}", retries + 1, e);
                    retries += 1;

                    if retries < self.reconnect_config.max_retries {
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(
                            delay.mul_f64(self.reconnect_config.backoff_multiplier),
                            self.reconnect_config.max_delay,
                        );
                    }
                },
            }
        }

        Err(PolyfillError::stream(
            format!(
                "Failed to reconnect after {} attempts",
                self.reconnect_config.max_retries
            ),
            StreamErrorKind::ConnectionFailed,
        ))
    }

    /// Get connection statistics (immutable reference)
    pub fn stats(&self) -> &StreamStats {
        &self.stats
    }

    /// Get connection statistics (mutable reference)
    pub fn stats_mut(&mut self) -> &mut StreamStats {
        &mut self.stats
    }

    /// Get the reconnect configuration
    pub fn reconnect_config(&self) -> &ReconnectConfig {
        &self.reconnect_config
    }

    /// Check if the connection appears stale (no activity within the given threshold)
    pub fn is_stale(&self, threshold: std::time::Duration) -> bool {
        match self.last_activity {
            Some(last) => last.elapsed() > threshold,
            None => false, // No activity yet means we haven't connected
        }
    }

    /// Get last activity time
    pub fn last_activity(&self) -> Option<std::time::Instant> {
        self.last_activity
    }

    /// Reset last activity to now (useful after reconnection)
    pub fn reset_activity(&mut self) {
        self.last_activity = Some(std::time::Instant::now());
    }

    /// Get mutable access to the underlying connection for polling
    ///
    /// This is needed for Stream trait implementations that need to poll
    /// the connection directly. Use with care - prefer `recv()` for most cases.
    pub fn connection_mut(&mut self) -> Option<&mut WsStream> {
        self.connection.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_creation() {
        let transport = WsTransport::new("wss://example.com/ws");
        assert_eq!(transport.url(), "wss://example.com/ws");
        assert!(!transport.is_connected());
        assert_eq!(transport.stats().messages_received, 0);
    }

    #[test]
    fn test_reconnect_config() {
        let config = ReconnectConfig {
            max_retries: 10,
            base_delay: std::time::Duration::from_millis(500),
            max_delay: std::time::Duration::from_secs(30),
            backoff_multiplier: 1.5,
            connect_timeout: std::time::Duration::from_secs(10),
            heartbeat_timeout: std::time::Duration::from_secs(60),
        };

        let transport = WsTransport::new("wss://example.com/ws").with_reconnect_config(config);

        assert_eq!(transport.reconnect_config().max_retries, 10);
        assert_eq!(
            transport.reconnect_config().base_delay,
            std::time::Duration::from_millis(500)
        );
    }
}
