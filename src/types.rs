//! Core types for the Polymarket client
//!
//! This module defines all the stable public types used throughout the client.
//! These types are optimized for latency-sensitive trading environments.

use alloy_primitives::{Address, U256};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Trading side for orders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    BUY = 0,
    SELL = 1,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::BUY => "BUY",
            Side::SELL => "SELL",
        }
    }

    pub fn opposite(&self) -> Self {
        match self {
            Side::BUY => Side::SELL,
            Side::SELL => Side::BUY,
        }
    }
}

/// Order type specifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderType {
    GTC,
    FOK,
    GTD,
}

impl OrderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderType::GTC => "GTC",
            OrderType::FOK => "FOK",
            OrderType::GTD => "GTD",
        }
    }
}

/// Order status in the system
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderStatus {
    #[serde(rename = "LIVE")]
    Live,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "FILLED")]
    Filled,
    #[serde(rename = "PARTIAL")]
    Partial,
    #[serde(rename = "EXPIRED")]
    Expired,
}

/// Market snapshot representing current state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub token_id: String,
    pub market_id: String,
    pub timestamp: DateTime<Utc>,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub mid: Option<Decimal>,
    pub spread: Option<Decimal>,
    pub last_price: Option<Decimal>,
    pub volume_24h: Option<Decimal>,
}

/// Order book level (price/size pair)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookLevel {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

/// Full order book state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    /// Token ID
    pub token_id: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Bid orders
    pub bids: Vec<BookLevel>,
    /// Ask orders
    pub asks: Vec<BookLevel>,
    /// Sequence number
    pub sequence: u64,
}

/// Order book delta for streaming updates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDelta {
    pub token_id: String,
    pub timestamp: DateTime<Utc>,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal, // 0 means remove level
    pub sequence: u64,
}

/// Trade execution event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillEvent {
    pub id: String,
    pub order_id: String,
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub timestamp: DateTime<Utc>,
    pub maker_address: Address,
    pub taker_address: Address,
    pub fee: Decimal,
}

/// Order creation parameters
#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal,
    pub order_type: OrderType,
    pub expiration: Option<DateTime<Utc>>,
    pub client_id: Option<String>,
}

/// Market order parameters
#[derive(Debug, Clone)]
pub struct MarketOrderRequest {
    pub token_id: String,
    pub side: Side,
    pub amount: Decimal, // USD amount for buys, token amount for sells
    pub slippage_tolerance: Option<Decimal>,
    pub client_id: Option<String>,
}

/// Order state in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub token_id: String,
    pub side: Side,
    pub price: Decimal,
    pub original_size: Decimal,
    pub filled_size: Decimal,
    pub remaining_size: Decimal,
    pub status: OrderStatus,
    pub order_type: OrderType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expiration: Option<DateTime<Utc>>,
    pub client_id: Option<String>,
}

/// API credentials for authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCredentials {
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

/// Configuration for order creation
#[derive(Debug, Clone)]
pub struct OrderOptions {
    pub tick_size: Option<Decimal>,
    pub neg_risk: Option<bool>,
    pub fee_rate_bps: Option<u32>,
}

/// Market information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub condition_id: String,
    pub tokens: [Token; 2],
    pub active: bool,
    pub closed: bool,
    pub question: String,
    pub description: String,
    pub category: Option<String>,
    pub end_date_iso: Option<String>,
    pub minimum_order_size: Decimal,
    pub minimum_tick_size: Decimal,
}

/// Token information within a market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_id: String,
    pub outcome: String,
}

/// Client configuration for PolyfillClient
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Base URL for the API
    pub base_url: String,
    /// Chain ID for the network
    pub chain_id: u64,
    /// Private key for signing (optional)
    pub private_key: Option<String>,
    /// API credentials (optional)
    pub api_credentials: Option<ApiCredentials>,
    /// Maximum slippage tolerance
    pub max_slippage: Option<Decimal>,
    /// Fee rate in basis points
    pub fee_rate: Option<Decimal>,
    /// Request timeout
    pub timeout: Option<std::time::Duration>,
    /// Maximum number of connections
    pub max_connections: Option<usize>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base_url: "https://clob.polymarket.com".to_string(),
            chain_id: 137, // Polygon mainnet
            private_key: None,
            api_credentials: None,
            timeout: Some(std::time::Duration::from_secs(30)),
            max_connections: Some(100),
            max_slippage: None,
            fee_rate: None,
        }
    }
}

/// WebSocket authentication for Polymarket API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssAuth {
    /// User's Ethereum address
    pub address: String,
    /// EIP-712 signature
    pub signature: String,
    /// Unix timestamp
    pub timestamp: u64,
    /// Nonce for replay protection
    pub nonce: String,
}

/// WebSocket subscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssSubscription {
    /// Authentication information
    pub auth: WssAuth,
    /// Array of markets (condition IDs) for USER channel
    pub markets: Option<Vec<String>>,
    /// Array of asset IDs (token IDs) for MARKET channel
    pub asset_ids: Option<Vec<String>>,
    /// Channel type: "USER" or "MARKET"
    #[serde(rename = "type")]
    pub channel_type: String,
}

/// WebSocket message types for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamMessage {
    #[serde(rename = "book_update")]
    BookUpdate {
        data: OrderDelta,
    },
    #[serde(rename = "trade")]
    Trade {
        data: FillEvent,
    },
    #[serde(rename = "order_update")]
    OrderUpdate {
        data: Order,
    },
    #[serde(rename = "heartbeat")]
    Heartbeat {
        timestamp: DateTime<Utc>,
    },
    /// User channel events
    #[serde(rename = "user_order_update")]
    UserOrderUpdate {
        data: Order,
    },
    #[serde(rename = "user_trade")]
    UserTrade {
        data: FillEvent,
    },
    /// Market channel events
    #[serde(rename = "market_book_update")]
    MarketBookUpdate {
        data: OrderDelta,
    },
    #[serde(rename = "market_trade")]
    MarketTrade {
        data: FillEvent,
    },
}

/// Subscription parameters for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub token_ids: Vec<String>,
    pub channels: Vec<String>,
}

/// WebSocket channel types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WssChannelType {
    #[serde(rename = "USER")]
    User,
    #[serde(rename = "MARKET")]
    Market,
}

impl WssChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WssChannelType::User => "USER",
            WssChannelType::Market => "MARKET",
        }
    }
}

/// Price quote response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub token_id: String,
    pub side: Side,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    pub timestamp: DateTime<Utc>,
}

/// Balance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub token_id: String,
    pub available: Decimal,
    pub locked: Decimal,
    pub total: Decimal,
}

/// Performance metrics for monitoring
#[derive(Debug, Clone)]
pub struct Metrics {
    pub orders_per_second: f64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
    pub uptime_pct: f64,
}

// Type aliases for common patterns
pub type TokenId = String;
pub type OrderId = String;
pub type MarketId = String;
pub type ClientId = String;

/// Result type used throughout the client
pub type Result<T> = std::result::Result<T, crate::errors::PolyfillError>; 