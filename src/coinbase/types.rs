//! Coinbase WebSocket message types
//!
//! Types for the Coinbase Exchange level2 orderbook feed.
//! Uses the same fixed-point optimization as the rest of polyfill-rs.

use crate::types::{Price, Qty, Side};
use serde::{Deserialize, Serialize};

/// WebSocket subscription message
#[derive(Debug, Clone, Serialize)]
pub struct Subscribe {
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub product_ids: Vec<String>,
    pub channels: Vec<&'static str>,
}

impl Subscribe {
    /// Create a level2 subscription for the given products
    /// Note: level2 now requires authentication on Coinbase Exchange
    pub fn level2(product_ids: Vec<String>) -> Self {
        Self {
            type_: "subscribe",
            product_ids,
            channels: vec!["level2", "heartbeat", "matches"],
        }
    }

    /// Create a level2_batch subscription for the given products
    /// This delivers updates in 50ms batches and does NOT require authentication
    pub fn level2_batch(product_ids: Vec<String>) -> Self {
        Self {
            type_: "subscribe",
            product_ids,
            channels: vec!["level2_batch", "heartbeat", "matches"],
        }
    }
}

/// Unsubscribe message
#[derive(Debug, Clone, Serialize)]
pub struct Unsubscribe {
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub product_ids: Vec<String>,
    pub channels: Vec<&'static str>,
}

impl Unsubscribe {
    pub fn level2(product_ids: Vec<String>) -> Self {
        Self {
            type_: "unsubscribe",
            product_ids,
            channels: vec!["level2", "heartbeat"],
        }
    }
}

/// Snapshot message - initial orderbook state
/// Received immediately after subscribing
#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub product_id: String,
    /// Array of [price, size] tuples
    pub bids: Vec<(String, String)>,
    /// Array of [price, size] tuples
    pub asks: Vec<(String, String)>,
}

/// L2 update message - incremental changes
/// Received after the initial snapshot
#[derive(Debug, Clone, Deserialize)]
pub struct L2Update {
    pub product_id: String,
    /// ISO 8601 timestamp
    pub time: String,
    /// Array of [side, price, size] tuples
    /// side is "buy" or "sell"
    /// size of "0" means remove the level
    pub changes: Vec<(String, String, String)>,
}

/// Heartbeat message - connection keep-alive
#[derive(Debug, Clone, Deserialize)]
pub struct Heartbeat {
    pub product_id: Option<String>,
    pub sequence: Option<u64>,
    pub time: Option<String>,
}

/// Subscriptions confirmation message
#[derive(Debug, Clone, Deserialize)]
pub struct Subscriptions {
    pub channels: Vec<SubscribedChannel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscribedChannel {
    pub name: String,
    pub product_ids: Vec<String>,
}

/// Match/trade message from the matches channel
#[derive(Debug, Clone, Deserialize)]
pub struct Match {
    pub trade_id: u64,
    pub product_id: String,
    pub time: String,
    pub size: String,
    pub price: String,
    pub side: String,
}

/// Error message from Coinbase
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessage {
    pub message: String,
    pub reason: Option<String>,
}

/// Parsed Coinbase WebSocket message
#[derive(Debug, Clone)]
pub enum Message {
    Snapshot(Snapshot),
    L2Update(L2Update),
    Match(Match),
    Heartbeat(Heartbeat),
    Subscriptions(Subscriptions),
    Error(ErrorMessage),
}

/// Fast internal representation of a price level update
/// Uses fixed-point integers for performance
#[derive(Debug, Clone, Copy)]
pub struct FastDelta {
    pub side: Side,
    pub price: Price,
    pub size: Qty,
}

/// Parsed snapshot in fast internal format
#[derive(Debug, Clone)]
pub struct FastSnapshot {
    pub product_id: String,
    pub bids: Vec<(Price, Qty)>,
    pub asks: Vec<(Price, Qty)>,
}

/// Parsed L2 update in fast internal format
#[derive(Debug, Clone)]
pub struct FastL2Update {
    pub product_id: String,
    pub changes: Vec<FastDelta>,
}
