//! Coinbase WebSocket stream for level2 orderbook data
//!
//! High-performance WebSocket implementation for Coinbase Exchange orderbook
//! with automatic reconnection and local orderbook state management.

use crate::book::OrderBook;
use crate::coinbase::decode::{l2update_to_fast, parse_message, snapshot_to_fast};
use crate::coinbase::types::{Message, Subscribe, Unsubscribe};
use crate::errors::Result;
use crate::stream::{ConnectionState, ReconnectConfig, ReconnectResult, StreamStats};
use crate::transport::WsTransport;
use crate::types::Side;
use chrono::Utc;
use futures::{Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Default Coinbase Exchange WebSocket URL
pub const DEFAULT_URL: &str = "wss://ws-feed.exchange.coinbase.com";

/// Coinbase level2 WebSocket stream
///
/// Maintains local orderbook state that is automatically updated from
/// the WebSocket feed. Provides the same performance optimizations as
/// the rest of polyfill-rs (fixed-point integers, SIMD parsing).
pub struct CoinbaseStream {
    /// WebSocket transport
    transport: WsTransport,
    /// Product IDs to subscribe to (e.g., ["BTC-USD"])
    product_ids: Vec<String>,
    /// Local orderbook state per product
    books: HashMap<String, OrderBook>,
    /// Maximum depth to maintain in orderbooks
    max_depth: usize,
    /// Message sender for internal communication (reserved for buffering)
    #[allow(dead_code)]
    tx: mpsc::UnboundedSender<Message>,
    /// Message receiver
    rx: mpsc::UnboundedReceiver<Message>,
    /// Whether we've received the initial snapshot
    has_snapshot: HashMap<String, bool>,
    /// Connection state for auto-reconnect
    connection_state: ConnectionState,
    /// Whether to use batch subscription (level2_batch vs level2)
    use_batch: bool,
}

impl CoinbaseStream {
    /// Create a new Coinbase stream for the given product IDs
    ///
    /// # Example
    /// ```rust,no_run
    /// use polyfill_rs::coinbase::CoinbaseStream;
    ///
    /// let stream = CoinbaseStream::new(vec!["BTC-USD".to_string()]);
    /// ```
    pub fn new(product_ids: Vec<String>) -> Self {
        Self::with_url(DEFAULT_URL, product_ids)
    }

    /// Create a new Coinbase stream with a custom URL
    pub fn with_url(url: &str, product_ids: Vec<String>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        let mut has_snapshot = HashMap::new();
        for id in &product_ids {
            has_snapshot.insert(id.clone(), false);
        }

        Self {
            transport: WsTransport::new(url),
            product_ids,
            books: HashMap::new(),
            max_depth: 100, // Default to 100 levels
            tx,
            rx,
            has_snapshot,
            connection_state: ConnectionState::Connected,
            use_batch: true, // Default to batch subscription (no auth required)
        }
    }

    /// Set the maximum orderbook depth to maintain
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Set custom reconnection configuration
    pub fn with_reconnect_config(mut self, config: ReconnectConfig) -> Self {
        self.transport = self.transport.with_reconnect_config(config);
        self
    }

    /// Connect to the WebSocket
    pub async fn connect(&mut self) -> Result<()> {
        self.transport.connect().await?;

        // Initialize orderbooks for each product
        for id in &self.product_ids {
            self.books
                .insert(id.clone(), OrderBook::new(id.clone(), self.max_depth));
            self.has_snapshot.insert(id.clone(), false);
        }

        Ok(())
    }

    /// Subscribe to level2 orderbook updates (requires authentication)
    ///
    /// Must be called within 5 seconds of connecting or the connection will be dropped.
    /// Note: level2 now requires authentication on Coinbase Exchange.
    /// Use `subscribe_batch()` for unauthenticated access.
    pub async fn subscribe(&mut self) -> Result<()> {
        if !self.transport.is_connected() {
            self.connect().await?;
        }

        let subscribe = Subscribe::level2(self.product_ids.clone());
        self.send_message(&subscribe).await?;
        self.use_batch = false;

        info!("Subscribed to level2 channel for {:?}", self.product_ids);
        Ok(())
    }

    /// Subscribe to level2_batch orderbook updates (no authentication required)
    ///
    /// Delivers updates in 50ms batches. This is the recommended method for
    /// unauthenticated access to orderbook data.
    pub async fn subscribe_batch(&mut self) -> Result<()> {
        if !self.transport.is_connected() {
            self.connect().await?;
        }

        let subscribe = Subscribe::level2_batch(self.product_ids.clone());
        self.send_message(&subscribe).await?;
        self.use_batch = true;

        info!(
            "Subscribed to level2_batch channel for {:?}",
            self.product_ids
        );
        Ok(())
    }

    /// Unsubscribe from level2 updates
    pub async fn unsubscribe(&mut self) -> Result<()> {
        let unsubscribe = Unsubscribe::level2(self.product_ids.clone());
        self.send_message(&unsubscribe).await?;
        info!("Unsubscribed from level2 channel");
        Ok(())
    }

    /// Send a message over the WebSocket
    async fn send_message<T: serde::Serialize>(&mut self, message: &T) -> Result<()> {
        self.transport.send(message).await
    }

    /// Get the current orderbook for a product
    ///
    /// Returns None if no snapshot has been received yet
    pub fn book(&self, product_id: &str) -> Option<&OrderBook> {
        if self.has_snapshot.get(product_id).copied().unwrap_or(false) {
            self.books.get(product_id)
        } else {
            None
        }
    }

    /// Get all orderbooks
    pub fn books(&self) -> &HashMap<String, OrderBook> {
        &self.books
    }

    /// Check if we have received the initial snapshot for a product
    pub fn has_snapshot(&self, product_id: &str) -> bool {
        self.has_snapshot.get(product_id).copied().unwrap_or(false)
    }

    /// Check if the stream is connected
    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    /// Get connection statistics
    pub fn get_stats(&self) -> &StreamStats {
        self.transport.stats()
    }

    /// Apply a snapshot to the local orderbook
    fn apply_snapshot(&mut self, product_id: &str, bids: &[(u32, i64)], asks: &[(u32, i64)]) {
        if let Some(book) = self.books.get_mut(product_id) {
            book.clear();

            for &(price, size) in bids {
                book.apply_level_fast(Side::BUY, price, size);
            }

            for &(price, size) in asks {
                book.apply_level_fast(Side::SELL, price, size);
            }

            self.has_snapshot.insert(product_id.to_string(), true);
            debug!(
                "Applied snapshot for {}: {} bids, {} asks",
                product_id,
                bids.len(),
                asks.len()
            );
        }
    }

    /// Apply an l2update to the local orderbook
    fn apply_l2update(&mut self, product_id: &str, changes: &[(Side, u32, i64)]) {
        if let Some(book) = self.books.get_mut(product_id) {
            for &(side, price, size) in changes {
                book.apply_level_fast(side, price, size);
            }
            debug!("Applied {} changes for {}", changes.len(), product_id);
        }
    }

    /// Handle an incoming WebSocket message
    fn handle_message(&mut self, text: &str) -> Result<Option<Message>> {
        let mut bytes = text.as_bytes().to_vec();
        let message = parse_message(&mut bytes)?;

        match &message {
            Message::Snapshot(snapshot) => match snapshot_to_fast(snapshot) {
                Ok(fast) => {
                    self.apply_snapshot(&fast.product_id, &fast.bids, &fast.asks);
                },
                Err(e) => {
                    warn!("Failed to parse snapshot: {}", e);
                },
            },
            Message::L2Update(update) => {
                // Only apply if we have a snapshot
                if self.has_snapshot(&update.product_id) {
                    match l2update_to_fast(update) {
                        Ok(fast) => {
                            let changes: Vec<_> = fast
                                .changes
                                .iter()
                                .map(|d| (d.side, d.price, d.size))
                                .collect();
                            self.apply_l2update(&fast.product_id, &changes);
                        },
                        Err(e) => {
                            warn!("Failed to parse l2update: {}", e);
                        },
                    }
                } else {
                    debug!(
                        "Ignoring l2update for {} (no snapshot yet)",
                        update.product_id
                    );
                }
            },
            Message::Heartbeat(_) => {
                debug!("Received heartbeat");
            },
            Message::Subscriptions(subs) => {
                info!("Subscribed to channels: {:?}", subs.channels);
            },
            Message::Error(err) => {
                error!("Coinbase error: {} ({:?})", err.message, err.reason);
            },
        }

        self.transport.stats_mut().messages_received += 1;
        self.transport.stats_mut().last_message_time = Some(Utc::now());

        Ok(Some(message))
    }

    /// Reconnect with exponential backoff
    pub async fn reconnect(&mut self) -> Result<()> {
        self.transport.reconnect().await?;

        // Clear snapshot flags - will get fresh snapshots
        for (_, has_snap) in self.has_snapshot.iter_mut() {
            *has_snap = false;
        }

        // Resubscribe based on previous subscription type
        if self.use_batch {
            self.subscribe_batch().await?;
        } else {
            self.subscribe().await?;
        }
        Ok(())
    }

    /// Start a reconnection attempt (for state machine)
    ///
    /// Creates a future that will reconnect and resubscribe to channels.
    fn start_reconnect(&mut self) {
        let url = self.transport.url().to_string();
        let config = self.transport.reconnect_config().clone();
        let product_ids = self.product_ids.clone();
        let use_batch = self.use_batch;

        info!("Starting Coinbase reconnection to {}", url);

        let future = Box::pin(async move {
            let mut transport = WsTransport::new(&url).with_reconnect_config(config);
            transport.reconnect().await?;

            // Subscribe to level2 or level2_batch
            let subscribe = if use_batch {
                Subscribe::level2_batch(product_ids.clone())
            } else {
                Subscribe::level2(product_ids.clone())
            };
            transport.send(&subscribe).await?;
            info!(
                "Resubscribed to {} channel for {:?}",
                if use_batch { "level2_batch" } else { "level2" },
                product_ids
            );

            Ok(ReconnectResult { transport })
        });

        self.connection_state = ConnectionState::Reconnecting(future);
    }
}

impl Stream for CoinbaseStream {
    type Item = Result<Message>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        use futures::SinkExt;

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
                            info!("Coinbase reconnection successful");
                            self.transport = result.transport;
                            // Clear snapshot flags - will get fresh snapshots
                            for (_, has_snap) in self.has_snapshot.iter_mut() {
                                *has_snap = false;
                            }
                            self.connection_state = ConnectionState::Connected;
                            continue;
                        },
                        Poll::Ready(Err(e)) => {
                            error!("Coinbase reconnection failed permanently: {}", e);
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
                    let heartbeat_timeout = self.transport.reconnect_config().heartbeat_timeout;
                    if self.transport.is_stale(heartbeat_timeout) {
                        warn!("Coinbase connection stale (no activity for {:?}), triggering reconnect", heartbeat_timeout);
                        self.transport.disconnect();
                        self.start_reconnect();
                        continue;
                    }

                    // Check WebSocket connection via transport
                    if let Some(connection) = self.transport.connection_mut() {
                        match connection.poll_next_unpin(cx) {
                            Poll::Ready(Some(Ok(ws_message))) => {
                                match ws_message {
                                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                                        self.transport.reset_activity();
                                        match self.handle_message(&text) {
                                            Ok(Some(msg)) => return Poll::Ready(Some(Ok(msg))),
                                            Ok(None) => {
                                                cx.waker().wake_by_ref();
                                                return Poll::Pending;
                                            },
                                            Err(e) => {
                                                self.transport.stats_mut().errors += 1;
                                                return Poll::Ready(Some(Err(e)));
                                            },
                                        }
                                    },
                                    tokio_tungstenite::tungstenite::Message::Ping(data) => {
                                        // Respond with pong
                                        self.transport.reset_activity();
                                        if let Some(conn) = self.transport.connection_mut() {
                                            let pong =
                                                tokio_tungstenite::tungstenite::Message::Pong(data);
                                            // Note: This is blocking but pong is small
                                            let _ = futures::executor::block_on(conn.send(pong));
                                        }
                                        cx.waker().wake_by_ref();
                                        return Poll::Pending;
                                    },
                                    tokio_tungstenite::tungstenite::Message::Pong(_) => {
                                        self.transport.reset_activity();
                                        cx.waker().wake_by_ref();
                                        return Poll::Pending;
                                    },
                                    tokio_tungstenite::tungstenite::Message::Close(_) => {
                                        info!("Coinbase WebSocket connection closed by server, triggering reconnect");
                                        self.transport.disconnect();
                                        self.start_reconnect();
                                        continue;
                                    },
                                    _ => {
                                        cx.waker().wake_by_ref();
                                        return Poll::Pending;
                                    },
                                }
                            },
                            Poll::Ready(Some(Err(e))) => {
                                warn!("Coinbase WebSocket error: {}, triggering reconnect", e);
                                self.transport.stats_mut().errors += 1;
                                self.transport.disconnect();
                                self.start_reconnect();
                                continue;
                            },
                            Poll::Ready(None) => {
                                info!("Coinbase WebSocket stream ended, triggering reconnect");
                                self.transport.disconnect();
                                self.start_reconnect();
                                continue;
                            },
                            Poll::Pending => return Poll::Pending,
                        }
                    } else {
                        // No connection - start reconnect
                        info!("No Coinbase WebSocket connection, triggering reconnect");
                        self.start_reconnect();
                        continue;
                    }
                },
            }
        }
    }
}
