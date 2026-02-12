//! Async streaming functionality for Polymarket client
//!
//! This module provides high-performance streaming capabilities for
//! real-time market data and order updates.

use crate::errors::{PolyfillError, Result};
use crate::transport::{RawMessage, WsTransport};
use crate::types::*;
use chrono::Utc;
use futures::{Future, Stream, StreamExt};
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
    /// WebSocket transport
    transport: WsTransport,
    /// Authentication credentials
    auth: Option<WssAuth>,
    /// Current subscriptions
    subscriptions: Vec<WssSubscription>,
    /// Message sender for internal communication
    tx: mpsc::UnboundedSender<StreamMessage>,
    /// Message receiver
    rx: mpsc::UnboundedReceiver<StreamMessage>,
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
    /// Timeout for initial connection attempts (default: 10 seconds)
    pub connect_timeout: std::time::Duration,
    /// Staleness threshold - trigger reconnect if no activity within this duration (default: 60 seconds)
    pub heartbeat_timeout: std::time::Duration,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: std::time::Duration::from_secs(1),
            max_delay: std::time::Duration::from_secs(60),
            backoff_multiplier: 2.0,
            connect_timeout: std::time::Duration::from_secs(10),
            heartbeat_timeout: std::time::Duration::from_secs(60),
        }
    }
}

/// Result of a successful reconnection attempt
pub struct ReconnectResult {
    /// New connected transport
    pub transport: WsTransport,
}

/// Type alias for the boxed reconnection future
type ReconnectFuture = Pin<Box<dyn Future<Output = Result<ReconnectResult>> + Send>>;

/// Connection state for resilient streams
pub enum ConnectionState {
    /// Connected and receiving messages
    Connected,
    /// Reconnection in progress
    Reconnecting(ReconnectFuture),
    /// Permanently failed after max retries
    Failed,
}

impl WebSocketStream {
    /// Create a new WebSocket stream
    pub fn new(url: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        Self {
            transport: WsTransport::new(url),
            auth: None,
            subscriptions: Vec::new(),
            tx,
            rx,
        }
    }

    /// Set authentication credentials
    pub fn with_auth(mut self, auth: WssAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Connect to the WebSocket
    async fn connect(&mut self) -> Result<()> {
        self.transport.connect().await
    }

    /// Send a message to the WebSocket
    async fn send_message(&mut self, message: Value) -> Result<()> {
        self.transport.send(&message).await
    }

    /// Subscribe to market data using official Polymarket WebSocket API
    pub async fn subscribe_async(&mut self, subscription: WssSubscription) -> Result<()> {
        // Ensure connection
        if !self.transport.is_connected() {
            self.connect().await?;
        }

        // Send subscription message in the format expected by Polymarket
        let message = serde_json::json!({
            "auth": subscription.auth,
            "markets": subscription.markets,
            "asset_ids": subscription.asset_ids,
            "type": subscription.channel_type,
        });

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
            auth,
            markets: Some(markets),
            asset_ids: None,
            channel_type: "USER".to_string(),
        };

        self.subscribe_async(subscription).await
    }

    /// Subscribe to market channel (order book and trades)
    pub async fn subscribe_market_channel(&mut self, asset_ids: Vec<String>) -> Result<()> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("No authentication provided for WebSocket"))?
            .clone();

        let subscription = WssSubscription {
            auth,
            markets: None,
            asset_ids: Some(asset_ids),
            channel_type: "MARKET".to_string(),
        };

        self.subscribe_async(subscription).await
    }

    /// Subscribe to public orderbook (no authentication required)
    ///
    /// Use with `wss://ws-subscriptions-clob.polymarket.com/ws/market`
    ///
    /// # Example
    /// ```rust,no_run
    /// use polyfill_rs::WsTransport;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut transport = WsTransport::new("wss://ws-subscriptions-clob.polymarket.com/ws/market");
    /// transport.connect().await?;
    ///
    /// // Subscribe to orderbook for specific assets
    /// let subscribe = serde_json::json!({
    ///     "assets_ids": ["token_id_1", "token_id_2"],
    ///     "type": "market"
    /// });
    /// transport.send(&subscribe).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_public_orderbook(&mut self, asset_ids: Vec<String>) -> Result<()> {
        // Ensure connection
        if !self.transport.is_connected() {
            self.connect().await?;
        }

        // Public orderbook uses different subscription format (no auth)
        let message = serde_json::json!({
            "assets_ids": asset_ids,
            "type": "market"
        });

        self.send_message(message).await?;
        info!(
            "Subscribed to public orderbook for {} assets",
            asset_ids.len()
        );
        Ok(())
    }

    /// Unsubscribe from market data
    pub async fn unsubscribe_async(&mut self, token_ids: &[String]) -> Result<()> {
        // Note: Polymarket WebSocket API doesn't seem to have explicit unsubscribe
        // We'll just remove from our local subscriptions
        self.subscriptions
            .retain(|sub| match sub.channel_type.as_str() {
                "USER" => {
                    if let Some(markets) = &sub.markets {
                        !token_ids.iter().any(|id| markets.contains(id))
                    } else {
                        true
                    }
                },
                "MARKET" => {
                    if let Some(asset_ids) = &sub.asset_ids {
                        !token_ids.iter().any(|id| asset_ids.contains(id))
                    } else {
                        true
                    }
                },
                _ => true,
            });

        info!("Unsubscribed from {} tokens", token_ids.len());
        Ok(())
    }

    /// Handle incoming WebSocket messages
    #[allow(dead_code)]
    fn handle_text_message(&mut self, text: &str) -> Result<()> {
        debug!("Received WebSocket message: {}", text);

        // Parse the message according to Polymarket's format
        let stream_message = self.parse_polymarket_message(text)?;

        // Send to internal channel
        if let Err(e) = self.tx.send(stream_message) {
            error!("Failed to send message to internal channel: {}", e);
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
        self.transport.reconnect().await?;

        // Resubscribe to all previous subscriptions
        let subscriptions = self.subscriptions.clone();
        for subscription in subscriptions {
            self.send_message(serde_json::to_value(subscription)?)
                .await?;
        }

        Ok(())
    }
}

impl Stream for WebSocketStream {
    type Item = Result<StreamMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // First check internal channel
        if let Poll::Ready(Some(message)) = self.rx.poll_recv(cx) {
            return Poll::Ready(Some(Ok(message)));
        }

        // Then check WebSocket connection via transport
        if let Some(connection) = self.transport.connection_mut() {
            match connection.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(_message))) => {
                    // Simplified message handling
                    Poll::Ready(Some(Ok(StreamMessage::Heartbeat {
                        timestamp: Utc::now(),
                    })))
                },
                Poll::Ready(Some(Err(e))) => {
                    error!("WebSocket error: {}", e);
                    self.transport.stats_mut().errors += 1;
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
        self.transport.is_connected()
    }

    fn get_stats(&self) -> StreamStats {
        self.transport.stats().clone()
    }
}

// ============================================================================
// LIVE DATA STREAM (wss://ws-live-data.polymarket.com)
// ============================================================================

use crate::types::{LiveDataMessage, LiveDataRequest, LiveDataSubscription, LiveTopic, Symbol};

/// Live data WebSocket stream for crypto prices and other feeds
pub struct LiveDataStream {
    /// WebSocket transport
    transport: WsTransport,
    /// Current subscriptions
    subscriptions: Vec<LiveDataSubscription>,
    /// Message sender for internal communication
    tx: mpsc::UnboundedSender<LiveDataMessage>,
    /// Message receiver
    rx: mpsc::UnboundedReceiver<LiveDataMessage>,
    /// Staleness threshold - if no message received within this duration, reconnect
    staleness_threshold: Option<std::time::Duration>,
    /// Last message time (for staleness check)
    last_message_instant: Option<std::time::Instant>,
    /// Connection state for auto-reconnect
    connection_state: ConnectionState,
}

impl LiveDataStream {
    /// Default URL for live data WebSocket
    pub const DEFAULT_URL: &'static str = "wss://ws-live-data.polymarket.com";

    /// Create a new live data stream with default URL
    pub fn new() -> Self {
        Self::with_url(Self::DEFAULT_URL)
    }

    /// Create a new live data stream with custom URL
    pub fn with_url(url: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        Self {
            transport: WsTransport::new(url),
            subscriptions: Vec::new(),
            tx,
            rx,
            staleness_threshold: None,
            last_message_instant: None,
            connection_state: ConnectionState::Connected,
        }
    }

    /// Set staleness threshold - stream will auto-reconnect if no message received within this duration
    pub fn with_staleness_threshold(mut self, threshold: std::time::Duration) -> Self {
        self.staleness_threshold = Some(threshold);
        self
    }

    /// Connect to the WebSocket
    pub async fn connect(&mut self) -> Result<()> {
        self.transport.connect().await?;
        self.last_message_instant = Some(std::time::Instant::now());
        Ok(())
    }

    /// Send a message to the WebSocket
    async fn send_message(&mut self, message: Value) -> Result<()> {
        self.transport.send(&message).await
    }

    /// Subscribe to a live data topic
    pub async fn subscribe(&mut self, subscription: LiveDataSubscription) -> Result<()> {
        // Ensure connection
        if !self.transport.is_connected() {
            self.connect().await?;
        }

        // Send subscription message
        let request = LiveDataRequest {
            action: "subscribe".to_string(),
            subscriptions: vec![subscription.clone()],
        };

        let message = serde_json::to_value(&request).map_err(|e| {
            PolyfillError::parse(format!("Failed to serialize subscription: {}", e), None)
        })?;

        self.send_message(message).await?;
        self.subscriptions.push(subscription.clone());

        info!("Subscribed to live data topic: {}", subscription.topic);
        Ok(())
    }

    /// Subscribe to a price feed (simplified API)
    pub async fn subscribe_price(&mut self, topic: LiveTopic, symbol: Symbol) -> Result<()> {
        self.subscribe(LiveDataSubscription::price(topic, symbol))
            .await
    }

    /// Unsubscribe from topics
    pub async fn unsubscribe(&mut self, topics: &[String]) -> Result<()> {
        let subs_to_remove: Vec<_> = self
            .subscriptions
            .iter()
            .filter(|s| topics.contains(&s.topic))
            .cloned()
            .collect();

        if !subs_to_remove.is_empty() {
            let request = LiveDataRequest {
                action: "unsubscribe".to_string(),
                subscriptions: subs_to_remove,
            };

            let message = serde_json::to_value(&request).map_err(|e| {
                PolyfillError::parse(format!("Failed to serialize unsubscription: {}", e), None)
            })?;

            self.send_message(message).await?;
        }

        self.subscriptions.retain(|s| !topics.contains(&s.topic));
        info!("Unsubscribed from {} topics", topics.len());
        Ok(())
    }

    /// Send a ping to keep the connection alive
    pub async fn ping(&mut self) -> Result<()> {
        self.transport.ping().await
    }

    /// Parse a live data message
    fn parse_message(&self, text: &str) -> Result<LiveDataMessage> {
        debug!("Raw WebSocket message: {}", text);
        serde_json::from_str(text).map_err(|e| {
            PolyfillError::parse(
                format!("Failed to parse live data message: {} (raw: {})", e, text),
                Some(Box::new(e)),
            )
        })
    }

    /// Check if the stream is connected
    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    /// Get connection statistics
    pub fn get_stats(&self) -> StreamStats {
        self.transport.stats().clone()
    }

    /// Check if the stream is stale (no messages within threshold)
    pub fn is_stale(&self) -> bool {
        match (self.staleness_threshold, self.last_message_instant) {
            (Some(threshold), Some(last)) => last.elapsed() > threshold,
            _ => false,
        }
    }

    /// Reconnect with exponential backoff, resubscribing to previous topics
    async fn reconnect(&mut self) -> Result<()> {
        self.transport.reconnect().await?;
        self.last_message_instant = Some(std::time::Instant::now());

        // Resubscribe to previous topics
        let subscriptions = self.subscriptions.clone();
        for subscription in subscriptions {
            if let Err(e) = self.subscribe(subscription).await {
                warn!("Failed to resubscribe: {}", e);
            }
        }

        Ok(())
    }

    /// Get the next message (convenience method for non-Stream usage)
    pub async fn next_message(&mut self) -> Option<Result<LiveDataMessage>> {
        // Check for staleness and reconnect if needed
        if self.is_stale() {
            warn!("Stream stale, triggering reconnect");
            if let Err(e) = self.reconnect().await {
                return Some(Err(e));
            }
        }

        // Use transport's recv which handles ping/pong automatically
        match self.transport.recv().await {
            Some(Ok(RawMessage::Text(text))) => {
                self.last_message_instant = Some(std::time::Instant::now());
                Some(self.parse_message(&text))
            },
            Some(Ok(RawMessage::Binary(_))) => {
                // Ignore binary messages, recurse
                Box::pin(self.next_message()).await
            },
            Some(Err(e)) => Some(Err(e)),
            None => None,
        }
    }
}

impl Default for LiveDataStream {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveDataStream {
    /// Start a reconnection attempt
    ///
    /// Creates a future that will reconnect and resubscribe to all topics.
    fn start_reconnect(&mut self) {
        let url = self.transport.url().to_string();
        let config = self.transport.reconnect_config().clone();
        let subscriptions = self.subscriptions.clone();

        info!("Starting reconnection to {}", url);

        let future = Box::pin(async move {
            let mut transport = WsTransport::new(&url).with_reconnect_config(config);
            transport.reconnect().await?;

            // Resubscribe to all previous topics
            for subscription in &subscriptions {
                let request = LiveDataRequest {
                    action: "subscribe".to_string(),
                    subscriptions: vec![subscription.clone()],
                };
                let message = serde_json::to_value(&request).map_err(|e| {
                    PolyfillError::parse(format!("Failed to serialize subscription: {}", e), None)
                })?;
                transport.send(&message).await?;
                info!("Resubscribed to topic: {}", subscription.topic);
            }

            Ok(ReconnectResult { transport })
        });

        self.connection_state = ConnectionState::Reconnecting(future);
    }
}

impl Stream for LiveDataStream {
    type Item = Result<LiveDataMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // Handle connection state machine
            match &mut self.connection_state {
                ConnectionState::Failed => {
                    return Poll::Ready(None);
                },

                ConnectionState::Reconnecting(future) => {
                    match future.as_mut().poll(cx) {
                        Poll::Ready(Ok(result)) => {
                            // Reconnect succeeded - swap in new transport
                            info!("Reconnection successful");
                            self.transport = result.transport;
                            self.last_message_instant = Some(std::time::Instant::now());
                            self.connection_state = ConnectionState::Connected;
                            continue;
                        },
                        Poll::Ready(Err(e)) => {
                            error!("Reconnection failed permanently: {}", e);
                            self.connection_state = ConnectionState::Failed;
                            return Poll::Ready(Some(Err(e)));
                        },
                        Poll::Pending => return Poll::Pending,
                    }
                },

                ConnectionState::Connected => {
                    // First check internal channel
                    if let Poll::Ready(Some(message)) = self.rx.poll_recv(cx) {
                        return Poll::Ready(Some(Ok(message)));
                    }

                    // Check for staleness
                    let heartbeat_timeout = self
                        .staleness_threshold
                        .unwrap_or(self.transport.reconnect_config().heartbeat_timeout);
                    if self.transport.is_stale(heartbeat_timeout) {
                        warn!(
                            "Connection stale (no activity for {:?}), triggering reconnect",
                            heartbeat_timeout
                        );
                        self.transport.disconnect();
                        self.start_reconnect();
                        continue;
                    }

                    // Check WebSocket connection via transport
                    if let Some(connection) = self.transport.connection_mut() {
                        match connection.poll_next_unpin(cx) {
                            Poll::Ready(Some(Ok(
                                tokio_tungstenite::tungstenite::Message::Text(text),
                            ))) => {
                                // Skip empty messages
                                if text.is_empty() {
                                    cx.waker().wake_by_ref();
                                    return Poll::Pending;
                                }
                                self.transport.stats_mut().messages_received += 1;
                                self.transport.stats_mut().last_message_time = Some(Utc::now());
                                self.transport.reset_activity();
                                self.last_message_instant = Some(std::time::Instant::now());
                                return Poll::Ready(Some(self.parse_message(&text)));
                            },
                            Poll::Ready(Some(Ok(
                                tokio_tungstenite::tungstenite::Message::Ping(data),
                            ))) => {
                                // Handle ping - send pong
                                self.transport.reset_activity();
                                if let Some(conn) = self.transport.connection_mut() {
                                    let pong = tokio_tungstenite::tungstenite::Message::Pong(data);
                                    let _ = futures::executor::block_on(futures::SinkExt::send(
                                        conn, pong,
                                    ));
                                }
                                cx.waker().wake_by_ref();
                                return Poll::Pending;
                            },
                            Poll::Ready(Some(Ok(
                                tokio_tungstenite::tungstenite::Message::Pong(_),
                            ))) => {
                                self.transport.reset_activity();
                                cx.waker().wake_by_ref();
                                return Poll::Pending;
                            },
                            Poll::Ready(Some(Ok(
                                tokio_tungstenite::tungstenite::Message::Close(_),
                            ))) => {
                                info!("LiveData WebSocket connection closed by server, triggering reconnect");
                                self.transport.disconnect();
                                self.start_reconnect();
                                continue;
                            },
                            Poll::Ready(Some(Ok(_))) => {
                                // Ignore other messages, wake to poll again
                                cx.waker().wake_by_ref();
                                return Poll::Pending;
                            },
                            Poll::Ready(Some(Err(e))) => {
                                warn!("WebSocket error: {}, triggering reconnect", e);
                                self.transport.stats_mut().errors += 1;
                                self.transport.disconnect();
                                self.start_reconnect();
                                continue;
                            },
                            Poll::Ready(None) => {
                                info!("WebSocket stream ended, triggering reconnect");
                                self.transport.disconnect();
                                self.start_reconnect();
                                continue;
                            },
                            Poll::Pending => return Poll::Pending,
                        }
                    } else {
                        // No connection - start reconnect
                        info!("No WebSocket connection, triggering reconnect");
                        self.start_reconnect();
                        continue;
                    }
                },
            }
        }
    }
}

// ============================================================================
// PUBLIC ORDERBOOK TYPES (wss://ws-subscriptions-clob.polymarket.com/ws/market)
// ============================================================================

/// Book update message from public orderbook WebSocket
///
/// The server can send messages in two formats:
/// - Snapshot: `{"asset_id": "...", "bids": [...], "asks": [...]}`
/// - Delta: `{"market": "...", "price_changes": [...]}`
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BookMessage {
    /// Asset ID (for snapshot messages)
    pub asset_id: Option<String>,
    /// Market ID (for delta messages) - used when asset_id is not present
    pub market: Option<String>,
    #[allow(dead_code)]
    pub event_type: Option<String>,
    #[allow(dead_code)]
    pub hash: Option<String>,
    #[allow(dead_code)]
    pub timestamp: Option<String>,
    /// Bids (snapshot)
    pub bids: Option<Vec<BookLevel>>,
    /// Asks (snapshot)
    pub asks: Option<Vec<BookLevel>>,
    /// Changes (snapshot delta format)
    pub changes: Option<Vec<BookChange>>,
    /// Price changes (delta message format)
    pub price_changes: Option<Vec<BookChange>>,
}

impl BookMessage {
    /// Get the asset/market identifier
    pub fn id(&self) -> Option<&str> {
        self.asset_id.as_deref().or(self.market.as_deref())
    }

    /// Get changes (handles both `changes` and `price_changes` fields)
    pub fn get_changes(&self) -> Option<&Vec<BookChange>> {
        self.changes.as_ref().or(self.price_changes.as_ref())
    }
}

/// Price level in orderbook
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BookLevel {
    pub price: String,
    pub size: String,
}

/// Orderbook change (delta update)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BookChange {
    /// Asset ID (for price_changes format)
    pub asset_id: Option<String>,
    pub side: String,
    pub price: String,
    pub size: String,
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
