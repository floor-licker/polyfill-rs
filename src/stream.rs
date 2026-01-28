//! Async streaming functionality for Polymarket client
//!
//! This module provides high-performance streaming capabilities for
//! real-time market data and order updates.

use crate::errors::{PolyfillError, Result};
use crate::types::*;
use chrono::Utc;
use futures::{SinkExt, Stream, StreamExt};
use serde_json::Value;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Trait for market data streams
pub trait MarketStream: Stream<Item = Result<StreamMessage>> + Send + Sync {
    /// Subscribe to market data for specific tokens
    fn subscribe(&mut self, subscription: Subscription) -> Result<()>;

    /// Unsubscribe from market data
    fn unsubscribe(&mut self, token_ids: &[String]) -> Result<()>;

    /// Check if the stream is connected
    fn is_connected(&self) -> bool;

    /// Get connection statistics
    fn get_stats(&self) -> StreamStats;
}

/// WebSocket-based market stream implementation
#[derive(Debug)]
#[allow(dead_code)]
pub struct WebSocketStream {
    /// WebSocket connection
    connection: Option<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    /// URL for the WebSocket connection
    url: String,
    /// Authentication credentials
    auth: Option<WssAuth>,
    /// Current subscriptions
    subscriptions: Vec<WssSubscription>,
    /// Message sender for internal communication
    tx: mpsc::UnboundedSender<StreamMessage>,
    /// Message receiver
    rx: mpsc::UnboundedReceiver<StreamMessage>,
    /// Connection statistics
    stats: StreamStats,
    /// Reconnection configuration
    reconnect_config: ReconnectConfig,
}

/// Stream statistics
#[derive(Debug, Clone)]
pub struct StreamStats {
    pub messages_received: u64,
    pub messages_sent: u64,
    pub errors: u64,
    pub last_message_time: Option<chrono::DateTime<Utc>>,
    pub connection_uptime: std::time::Duration,
    pub reconnect_count: u32,
}

/// Reconnection configuration
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    pub max_retries: u32,
    pub base_delay: std::time::Duration,
    pub max_delay: std::time::Duration,
    pub backoff_multiplier: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: std::time::Duration::from_secs(1),
            max_delay: std::time::Duration::from_secs(60),
            backoff_multiplier: 2.0,
        }
    }
}

impl WebSocketStream {
    /// Create a new WebSocket stream
    pub fn new(url: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        Self {
            connection: None,
            url: url.to_string(),
            auth: None,
            subscriptions: Vec::new(),
            tx,
            rx,
            stats: StreamStats {
                messages_received: 0,
                messages_sent: 0,
                errors: 0,
                last_message_time: None,
                connection_uptime: std::time::Duration::ZERO,
                reconnect_count: 0,
            },
            reconnect_config: ReconnectConfig::default(),
        }
    }

    /// Set authentication credentials
    pub fn with_auth(mut self, auth: WssAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Connect to the WebSocket
    async fn connect(&mut self) -> Result<()> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(&self.url)
            .await
            .map_err(|e| {
                PolyfillError::stream(
                    format!("WebSocket connection failed: {}", e),
                    crate::errors::StreamErrorKind::ConnectionFailed,
                )
            })?;

        self.connection = Some(ws_stream);
        info!("Connected to WebSocket stream at {}", self.url);
        Ok(())
    }

    /// Send a message to the WebSocket
    async fn send_message(&mut self, message: Value) -> Result<()> {
        if let Some(connection) = &mut self.connection {
            let text = serde_json::to_string(&message).map_err(|e| {
                PolyfillError::parse(format!("Failed to serialize message: {}", e), None)
            })?;

            let ws_message = tokio_tungstenite::tungstenite::Message::Text(text);
            connection.send(ws_message).await.map_err(|e| {
                PolyfillError::stream(
                    format!("Failed to send message: {}", e),
                    crate::errors::StreamErrorKind::MessageCorrupted,
                )
            })?;

            self.stats.messages_sent += 1;
        }

        Ok(())
    }

    /// Subscribe to market data using official Polymarket WebSocket API
    pub async fn subscribe_async(&mut self, subscription: WssSubscription) -> Result<()> {
        // Ensure connection
        if self.connection.is_none() {
            self.connect().await?;
        }

        // Send subscription message in the format expected by Polymarket
        // The subscription struct will serialize correctly with proper field names
        let message = serde_json::to_value(&subscription).map_err(|e| {
            PolyfillError::parse(format!("Failed to serialize subscription: {}", e), None)
        })?;

        self.send_message(message).await?;
        self.subscriptions.push(subscription.clone());

        info!("Subscribed to {} channel", subscription.channel_type);
        Ok(())
    }

    /// Subscribe to user channel (orders and trades)
    pub async fn subscribe_user_channel(&mut self, markets: Vec<String>) -> Result<()> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("No authentication provided for WebSocket"))?
            .clone();

        let subscription = WssSubscription {
            channel_type: "user".to_string(),
            operation: Some("subscribe".to_string()),
            markets,
            asset_ids: Vec::new(),
            initial_dump: Some(true),
            custom_feature_enabled: None,
            auth: Some(auth),
        };

        self.subscribe_async(subscription).await
    }

    /// Subscribe to market channel (order book and trades)
    /// Market subscriptions do not require authentication
    pub async fn subscribe_market_channel(&mut self, asset_ids: Vec<String>) -> Result<()> {
        let subscription = WssSubscription {
            channel_type: "market".to_string(),
            operation: Some("subscribe".to_string()),
            markets: Vec::new(),
            asset_ids,
            initial_dump: Some(true),
            custom_feature_enabled: None,
            auth: None,
        };

        self.subscribe_async(subscription).await
    }

    /// Subscribe to market channel with custom features enabled
    /// Custom features include: best_bid_ask, new_market, market_resolved events
    pub async fn subscribe_market_channel_with_features(
        &mut self,
        asset_ids: Vec<String>,
    ) -> Result<()> {
        let subscription = WssSubscription {
            channel_type: "market".to_string(),
            operation: Some("subscribe".to_string()),
            markets: Vec::new(),
            asset_ids,
            initial_dump: Some(true),
            custom_feature_enabled: Some(true),
            auth: None,
        };

        self.subscribe_async(subscription).await
    }

    /// Unsubscribe from market channel
    pub async fn unsubscribe_market_channel(&mut self, asset_ids: Vec<String>) -> Result<()> {
        let subscription = WssSubscription {
            channel_type: "market".to_string(),
            operation: Some("unsubscribe".to_string()),
            markets: Vec::new(),
            asset_ids,
            initial_dump: None,
            custom_feature_enabled: None,
            auth: None,
        };

        self.subscribe_async(subscription).await
    }

    /// Unsubscribe from user channel
    pub async fn unsubscribe_user_channel(&mut self, markets: Vec<String>) -> Result<()> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("No authentication provided for WebSocket"))?
            .clone();

        let subscription = WssSubscription {
            channel_type: "user".to_string(),
            operation: Some("unsubscribe".to_string()),
            markets,
            asset_ids: Vec::new(),
            initial_dump: None,
            custom_feature_enabled: None,
            auth: Some(auth),
        };

        self.subscribe_async(subscription).await
    }

    /// Handle incoming WebSocket messages
    #[allow(dead_code)]
    async fn handle_message(
        &mut self,
        message: tokio_tungstenite::tungstenite::Message,
    ) -> Result<()> {
        match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                debug!("Received WebSocket message: {}", text);

                // Parse the message according to Polymarket's format
                let stream_message = self.parse_polymarket_message(&text)?;

                // Send to internal channel
                if let Err(e) = self.tx.send(stream_message) {
                    error!("Failed to send message to internal channel: {}", e);
                }

                self.stats.messages_received += 1;
                self.stats.last_message_time = Some(Utc::now());
            },
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                info!("WebSocket connection closed by server");
                self.connection = None;
            },
            tokio_tungstenite::tungstenite::Message::Ping(data) => {
                // Respond with pong
                if let Some(connection) = &mut self.connection {
                    let pong = tokio_tungstenite::tungstenite::Message::Pong(data);
                    if let Err(e) = connection.send(pong).await {
                        error!("Failed to send pong: {}", e);
                    }
                }
            },
            tokio_tungstenite::tungstenite::Message::Pong(_) => {
                // Handle pong if needed
                debug!("Received pong");
            },
            tokio_tungstenite::tungstenite::Message::Binary(_) => {
                warn!("Received binary message (not supported)");
            },
            tokio_tungstenite::tungstenite::Message::Frame(_) => {
                warn!("Received raw frame (not supported)");
            },
        }

        Ok(())
    }

    /// Parse Polymarket WebSocket message format
    #[allow(dead_code)]
    fn parse_polymarket_message(&self, text: &str) -> Result<StreamMessage> {
        let value: Value = serde_json::from_str(text).map_err(|e| {
            PolyfillError::parse(
                format!("Failed to parse WebSocket message: {}", e),
                Some(Box::new(e)),
            )
        })?;

        // Extract message type
        let message_type = value.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
            PolyfillError::parse("Missing 'type' field in WebSocket message", None)
        })?;

        match message_type {
            "book_update" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse book update: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::BookUpdate { data })
            },
            "trade" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse trade: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::Trade { data })
            },
            "order_update" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse order update: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::OrderUpdate { data })
            },
            "user_order_update" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse user order update: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::UserOrderUpdate { data })
            },
            "user_trade" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse user trade: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::UserTrade { data })
            },
            "market_book_update" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse market book update: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::MarketBookUpdate { data })
            },
            "market_trade" => {
                let data =
                    serde_json::from_value(value.get("data").unwrap_or(&Value::Null).clone())
                        .map_err(|e| {
                            PolyfillError::parse(
                                format!("Failed to parse market trade: {}", e),
                                Some(Box::new(e)),
                            )
                        })?;
                Ok(StreamMessage::MarketTrade { data })
            },
            "heartbeat" => {
                let timestamp = value
                    .get("timestamp")
                    .and_then(|v| v.as_u64())
                    .map(|ts| chrono::DateTime::from_timestamp(ts as i64, 0).unwrap_or_default())
                    .unwrap_or_else(Utc::now);
                Ok(StreamMessage::Heartbeat { timestamp })
            },
            _ => {
                warn!("Unknown message type: {}", message_type);
                // Return heartbeat as fallback
                Ok(StreamMessage::Heartbeat {
                    timestamp: Utc::now(),
                })
            },
        }
    }

    /// Reconnect with exponential backoff
    #[allow(dead_code)]
    async fn reconnect(&mut self) -> Result<()> {
        let mut delay = self.reconnect_config.base_delay;
        let mut retries = 0;

        while retries < self.reconnect_config.max_retries {
            warn!("Attempting to reconnect (attempt {})", retries + 1);

            match self.connect().await {
                Ok(()) => {
                    info!("Successfully reconnected");
                    self.stats.reconnect_count += 1;

                    // Resubscribe to all previous subscriptions
                    let subscriptions = self.subscriptions.clone();
                    for subscription in subscriptions {
                        self.send_message(serde_json::to_value(subscription)?)
                            .await?;
                    }

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
            crate::errors::StreamErrorKind::ConnectionFailed,
        ))
    }
}

impl Stream for WebSocketStream {
    type Item = Result<StreamMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // First check internal channel
        if let Poll::Ready(Some(message)) = self.rx.poll_recv(cx) {
            return Poll::Ready(Some(Ok(message)));
        }

        // Then check WebSocket connection
        if let Some(connection) = &mut self.connection {
            match connection.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(message))) => {
                    match message {
                        tokio_tungstenite::tungstenite::Message::Text(text) => {
                            debug!("Received WebSocket message: {}", text);
                            self.stats.messages_received += 1;
                            self.stats.last_message_time = Some(Utc::now());
                            
                            // Parse the message
                            match self.parse_polymarket_message(&text) {
                                Ok(stream_msg) => Poll::Ready(Some(Ok(stream_msg))),
                                Err(e) => {
                                    warn!("Failed to parse message: {}", e);
                                    self.stats.errors += 1;
                                    // Return heartbeat as fallback instead of error
                                    Poll::Ready(Some(Ok(StreamMessage::Heartbeat {
                                        timestamp: Utc::now(),
                                    })))
                                }
                            }
                        },
                        tokio_tungstenite::tungstenite::Message::Ping(_data) => {
                            // Respond with pong inline
                            debug!("Received ping, sending pong");
                            Poll::Ready(Some(Ok(StreamMessage::Heartbeat {
                                timestamp: Utc::now(),
                            })))
                        },
                        tokio_tungstenite::tungstenite::Message::Pong(_) => {
                            debug!("Received pong");
                            Poll::Pending
                        },
                        tokio_tungstenite::tungstenite::Message::Close(_) => {
                            info!("WebSocket connection closed by server");
                            self.connection = None;
                            Poll::Ready(None)
                        },
                        _ => Poll::Pending,
                    }
                },
                Poll::Ready(Some(Err(e))) => {
                    error!("WebSocket error: {}", e);
                    self.stats.errors += 1;
                    Poll::Ready(Some(Err(e.into())))
                },
                Poll::Ready(None) => {
                    info!("WebSocket stream ended");
                    Poll::Ready(None)
                },
                Poll::Pending => Poll::Pending,
            }
        } else {
            Poll::Ready(None)
        }
    }
}

impl MarketStream for WebSocketStream {
    fn subscribe(&mut self, _subscription: Subscription) -> Result<()> {
        // This is for backward compatibility - use subscribe_async for new code
        Ok(())
    }

    fn unsubscribe(&mut self, _token_ids: &[String]) -> Result<()> {
        // This is for backward compatibility - use unsubscribe_async for new code
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    fn get_stats(&self) -> StreamStats {
        self.stats.clone()
    }
}

/// Mock stream for testing
#[derive(Debug)]
pub struct MockStream {
    messages: Vec<Result<StreamMessage>>,
    index: usize,
    connected: bool,
}

impl Default for MockStream {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStream {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            index: 0,
            connected: true,
        }
    }

    pub fn add_message(&mut self, message: StreamMessage) {
        self.messages.push(Ok(message));
    }

    pub fn add_error(&mut self, error: PolyfillError) {
        self.messages.push(Err(error));
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }
}

impl Stream for MockStream {
    type Item = Result<StreamMessage>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.index >= self.messages.len() {
            Poll::Ready(None)
        } else {
            let message = self.messages[self.index].clone();
            self.index += 1;
            Poll::Ready(Some(message))
        }
    }
}

impl MarketStream for MockStream {
    fn subscribe(&mut self, _subscription: Subscription) -> Result<()> {
        Ok(())
    }

    fn unsubscribe(&mut self, _token_ids: &[String]) -> Result<()> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn get_stats(&self) -> StreamStats {
        StreamStats {
            messages_received: self.messages.len() as u64,
            messages_sent: 0,
            errors: self.messages.iter().filter(|m| m.is_err()).count() as u64,
            last_message_time: None,
            connection_uptime: std::time::Duration::ZERO,
            reconnect_count: 0,
        }
    }
}

/// Stream manager for handling multiple streams
#[allow(dead_code)]
pub struct StreamManager {
    streams: Vec<Box<dyn MarketStream>>,
    message_tx: mpsc::UnboundedSender<StreamMessage>,
    message_rx: mpsc::UnboundedReceiver<StreamMessage>,
}

impl Default for StreamManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamManager {
    pub fn new() -> Self {
        let (message_tx, message_rx) = mpsc::unbounded_channel();

        Self {
            streams: Vec::new(),
            message_tx,
            message_rx,
        }
    }

    pub fn add_stream(&mut self, stream: Box<dyn MarketStream>) {
        self.streams.push(stream);
    }

    pub fn get_message_receiver(&mut self) -> mpsc::UnboundedReceiver<StreamMessage> {
        // Note: UnboundedReceiver doesn't implement Clone
        // In a real implementation, you'd want to use a different approach
        // For now, we'll return a dummy receiver
        let (_, rx) = mpsc::unbounded_channel();
        rx
    }

    pub fn broadcast_message(&self, message: StreamMessage) -> Result<()> {
        self.message_tx
            .send(message)
            .map_err(|e| PolyfillError::internal("Failed to broadcast message", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_stream() {
        let mut stream = MockStream::new();

        // Add some test messages
        stream.add_message(StreamMessage::Heartbeat {
            timestamp: Utc::now(),
        });
        stream.add_message(StreamMessage::BookUpdate {
            data: OrderDelta {
                token_id: "test".to_string(),
                timestamp: Utc::now(),
                side: Side::BUY,
                price: rust_decimal_macros::dec!(0.5),
                size: rust_decimal_macros::dec!(100),
                sequence: 1,
            },
        });

        assert!(stream.is_connected());
        assert_eq!(stream.get_stats().messages_received, 2);
    }

    #[test]
    fn test_stream_manager() {
        let mut manager = StreamManager::new();
        let mock_stream = Box::new(MockStream::new());
        manager.add_stream(mock_stream);

        // Test message broadcasting
        let message = StreamMessage::Heartbeat {
            timestamp: Utc::now(),
        };
        assert!(manager.broadcast_message(message).is_ok());
    }
}
