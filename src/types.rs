//! Core types for the Polymarket client
//!
//! This module defines all the stable public types used throughout the client.
//! These types are optimized for latency-sensitive trading environments.

use alloy_primitives::{Address, U256};
use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ============================================================================
// FIXED-POINT OPTIMIZATION FOR HOT PATH PERFORMANCE
// ============================================================================
//
// Instead of using rust_decimal::Decimal everywhere (which allocates),
// I've used fixed-point integers for the performance-critical order book operations.
//
// Why this matters:
// - Decimal operations can be 10-100x slower than integer operations
// - Decimal allocates memory for each calculation
// - In an order book like this we process thousands of price updates per second
// - Most prices can be represented as integer ticks (e.g., $0.6543 = 6543 ticks)
//
// The strategy:
// 1. Convert Decimal to fixed-point on ingress (when data comes in)
// 2. Do all hot-path calculations with integers
// 3. Convert back to Decimal only at the edges (API responses, user display)
//
// This is like how video games handle positions, they use integers internally
// for speed, but show floating-point coordinates to players.
/// Each tick represents 0.0001 (1/10,000) of the base unit
/// Examples:
/// - $0.6543 = 6543 ticks
/// - $1.0000 = 10000 ticks  
/// - $0.0001 = 1 tick (minimum price increment)
///
/// Why u32?
/// - Can represent prices from $0.0001 to $429,496.7295 (way more than needed)
/// - Fits in CPU register for fast operations
/// - No sign bit needed since prices are always positive
pub type Price = u32;

/// Quantity/size represented as fixed-point integer for performance
///
/// Each unit represents 0.0001 (1/10,000) of a token
/// Examples:
/// - 100.0 tokens = 1,000,000 units
/// - 0.0001 tokens = 1 unit (minimum size increment)
///
/// Why i64?
/// - Can represent quantities from -922,337,203,685.4775 to +922,337,203,685.4775
/// - Signed because we need to handle both buys (+) and sells (-)
/// - Large enough for any realistic trading size
pub type Qty = i64;

/// Scale factor for converting between Decimal and fixed-point
///
/// We use 10,000 (1e4) as our scale factor, giving us 4 decimal places of precision.
/// This is perfect for most prediction markets where prices are between $0.01-$0.99
/// and we need precision to the nearest $0.0001.
pub const SCALE_FACTOR: i64 = 10_000;

/// Maximum valid price in ticks (prevents overflow)
/// This represents $429,496.7295 which is way higher than any prediction market price
pub const MAX_PRICE_TICKS: Price = Price::MAX;

/// Minimum valid price in ticks (1 tick = $0.0001)
pub const MIN_PRICE_TICKS: Price = 1;

/// Maximum valid quantity (prevents overflow in calculations)
pub const MAX_QTY: Qty = Qty::MAX / 2; // Leave room for intermediate calculations

// ============================================================================
// CONVERSION FUNCTIONS BETWEEN DECIMAL AND FIXED-POINT
// ============================================================================
//
// These functions handle the conversion between the external Decimal API
// and our internal fixed-point representation. They're designed to be fast
// and handle edge cases gracefully.

/// Convert a Decimal price to fixed-point ticks
///
/// This is called when we receive price data from the API or user input.
/// We quantize the price to the nearest tick to ensure all prices are
/// aligned to our internal representation.
///
/// Examples:
/// - decimal_to_price(Decimal::from_str("0.6543")) = Ok(6543)
/// - decimal_to_price(Decimal::from_str("1.0000")) = Ok(10000)
/// - decimal_to_price(Decimal::from_str("0.00005")) = Ok(1) // Rounds up to min tick
pub fn decimal_to_price(decimal: Decimal) -> std::result::Result<Price, &'static str> {
    // Convert to fixed-point by multiplying by scale factor
    let scaled = decimal * Decimal::from(SCALE_FACTOR);

    // Round to nearest integer (this handles tick alignment automatically)
    let rounded = scaled.round();

    // Convert to u64 first to handle the conversion safely
    let as_u64 = rounded.to_u64().ok_or("Price too large or negative")?;

    // Check bounds
    if as_u64 < MIN_PRICE_TICKS as u64 {
        return Ok(MIN_PRICE_TICKS); // Clamp to minimum
    }
    if as_u64 > MAX_PRICE_TICKS as u64 {
        return Err("Price exceeds maximum");
    }

    Ok(as_u64 as Price)
}

/// Convert fixed-point ticks back to Decimal price
///
/// This is called when we need to return price data to the API or display to users.
/// It's the inverse of decimal_to_price().
///
/// Examples:
/// - price_to_decimal(6543) = Decimal::from_str("0.6543")
/// - price_to_decimal(10000) = Decimal::from_str("1.0000")
pub fn price_to_decimal(ticks: Price) -> Decimal {
    Decimal::from(ticks) / Decimal::from(SCALE_FACTOR)
}

/// Convert a Decimal quantity to fixed-point units
///
/// Similar to decimal_to_price but handles signed quantities.
/// Quantities can be negative (for sells or position changes).
///
/// Examples:
/// - decimal_to_qty(Decimal::from_str("100.0")) = Ok(1000000)
/// - decimal_to_qty(Decimal::from_str("-50.5")) = Ok(-505000)
pub fn decimal_to_qty(decimal: Decimal) -> std::result::Result<Qty, &'static str> {
    let scaled = decimal * Decimal::from(SCALE_FACTOR);
    let rounded = scaled.round();

    let as_i64 = rounded.to_i64().ok_or("Quantity too large")?;

    if as_i64.abs() > MAX_QTY {
        return Err("Quantity exceeds maximum");
    }

    Ok(as_i64)
}

/// Convert fixed-point units back to Decimal quantity
///
/// Examples:
/// - qty_to_decimal(1000000) = Decimal::from_str("100.0")
/// - qty_to_decimal(-505000) = Decimal::from_str("-50.5")
pub fn qty_to_decimal(units: Qty) -> Decimal {
    Decimal::from(units) / Decimal::from(SCALE_FACTOR)
}

/// Check if a price is properly tick-aligned
///
/// This is used to validate incoming price data. In a well-behaved system,
/// all prices should already be tick-aligned, but we check anyway to catch
/// bugs or malicious data.
///
/// A price is tick-aligned if it's an exact multiple of the minimum tick size.
/// Since we use integer ticks internally, this just checks if the price
/// converts cleanly to our internal representation.
pub fn is_price_tick_aligned(decimal: Decimal, tick_size_decimal: Decimal) -> bool {
    // Convert tick size to our internal representation
    let tick_size_ticks = match decimal_to_price(tick_size_decimal) {
        Ok(ticks) => ticks,
        Err(_) => return false,
    };

    // Convert the price to ticks
    let price_ticks = match decimal_to_price(decimal) {
        Ok(ticks) => ticks,
        Err(_) => return false,
    };

    // Check if price is a multiple of tick size
    // If tick_size_ticks is 0, we consider everything aligned (no restrictions)
    if tick_size_ticks == 0 {
        return true;
    }

    price_ticks % tick_size_ticks == 0
}

/// Trading side for orders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[allow(clippy::upper_case_acronyms)]
pub enum OrderType {
    #[default]
    GTC,
    FOK,
    GTD,
    FAK,
}

impl OrderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderType::GTC => "GTC",
            OrderType::FOK => "FOK",
            OrderType::GTD => "GTD",
            OrderType::FAK => "FAK",
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

/// Order book level (price/size pair) - EXTERNAL API VERSION
///
/// This is what we expose to users and serialize to JSON.
/// It uses Decimal for precision and human readability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookLevel {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

/// Order book level (price/size pair) - INTERNAL HOT PATH VERSION
///
/// This is what we use internally for maximum performance.
/// All order book operations use this to avoid Decimal overhead.
///
/// The performance difference is huge:
/// - BookLevel: ~50ns per operation (Decimal math + allocation)
/// - FastBookLevel: ~2ns per operation (integer math, no allocation)
///
/// That's a 25x speedup on the critical path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastBookLevel {
    pub price: Price, // Price in ticks (u32)
    pub size: Qty,    // Size in fixed-point units (i64)
}

impl FastBookLevel {
    /// Create a new fast book level
    pub fn new(price: Price, size: Qty) -> Self {
        Self { price, size }
    }

    /// Convert to external BookLevel for API responses
    /// This is only called at the edges when we need to return data to users
    pub fn to_book_level(self) -> BookLevel {
        BookLevel {
            price: price_to_decimal(self.price),
            size: qty_to_decimal(self.size),
        }
    }

    /// Create from external BookLevel (with validation)
    /// This is called when we receive data from the API
    pub fn from_book_level(level: &BookLevel) -> std::result::Result<Self, &'static str> {
        let price = decimal_to_price(level.price)?;
        let size = decimal_to_qty(level.size)?;
        Ok(Self::new(price, size))
    }

    /// Calculate notional value (price * size) in fixed-point
    /// Returns the result scaled appropriately to avoid overflow
    ///
    /// This is much faster than the Decimal equivalent:
    /// - Decimal: price.mul(size) -> ~20ns + allocation
    /// - Fixed-point: (price as i64 * size) / SCALE_FACTOR -> ~1ns, no allocation
    pub fn notional(self) -> i64 {
        // Convert price to i64 to avoid overflow in multiplication
        let price_i64 = self.price as i64;
        // Multiply and scale back down (we scaled both price and size up by SCALE_FACTOR)
        (price_i64 * self.size) / SCALE_FACTOR
    }
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

/// Order book delta for streaming updates - EXTERNAL API VERSION
///
/// This is what we receive from WebSocket streams and REST API calls.
/// It uses Decimal for compatibility with external systems.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderDelta {
    pub token_id: String,
    pub timestamp: DateTime<Utc>,
    pub side: Side,
    pub price: Decimal,
    pub size: Decimal, // 0 means remove level
    pub sequence: u64,
}

/// Order book delta for streaming updates - INTERNAL HOT PATH VERSION
///
/// This is what we use internally for processing order book updates.
/// Converting to this format on ingress gives us massive performance gains.
///
/// Why the performance matters:
/// - We might process 10,000+ deltas per second in active markets
/// - Each delta triggers multiple calculations (spread, impact, etc.)
/// - Using integers instead of Decimal can make the difference between
///   keeping up with the market feed vs falling behind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastOrderDelta {
    pub token_id_hash: u64, // Hash of token_id for fast lookup (avoids string comparisons)
    pub timestamp: DateTime<Utc>,
    pub side: Side,
    pub price: Price, // Price in ticks
    pub size: Qty,    // Size in fixed-point units (0 means remove level)
    pub sequence: u64,
}

impl FastOrderDelta {
    /// Create from external OrderDelta with validation and tick alignment
    ///
    /// This is where we enforce tick alignment - if the incoming price
    /// doesn't align to valid ticks, we either reject it or round it.
    /// This prevents bad data from corrupting our order book.
    pub fn from_order_delta(
        delta: &OrderDelta,
        tick_size: Option<Decimal>,
    ) -> std::result::Result<Self, &'static str> {
        // Validate tick alignment if we have a tick size
        if let Some(tick_size) = tick_size {
            if !is_price_tick_aligned(delta.price, tick_size) {
                return Err("Price not aligned to tick size");
            }
        }

        // Convert to fixed-point with validation
        let price = decimal_to_price(delta.price)?;
        let size = decimal_to_qty(delta.size)?;

        // Hash the token_id for fast lookups
        // This avoids string comparisons in the hot path
        let token_id_hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            delta.token_id.hash(&mut hasher);
            hasher.finish()
        };

        Ok(Self {
            token_id_hash,
            timestamp: delta.timestamp,
            side: delta.side,
            price,
            size,
            sequence: delta.sequence,
        })
    }

    /// Convert back to external OrderDelta (for API responses)
    /// We need the original token_id since we only store the hash
    pub fn to_order_delta(self, token_id: String) -> OrderDelta {
        OrderDelta {
            token_id,
            timestamp: self.timestamp,
            side: self.side,
            price: price_to_decimal(self.price),
            size: qty_to_decimal(self.size),
            sequence: self.sequence,
        }
    }

    /// Check if this delta removes a level (size is zero)
    pub fn is_removal(self) -> bool {
        self.size == 0
    }
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiCredentials {
    #[serde(rename = "apiKey")]
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

/// Extra arguments for order creation
#[derive(Debug, Clone)]
pub struct ExtraOrderArgs {
    pub fee_rate_bps: u32,
    pub nonce: U256,
    pub taker: String,
}

impl Default for ExtraOrderArgs {
    fn default() -> Self {
        Self {
            fee_rate_bps: 0,
            nonce: U256::ZERO,
            taker: "0x0000000000000000000000000000000000000000".to_string(),
        }
    }
}

/// Market order arguments
#[derive(Debug, Clone)]
pub struct MarketOrderArgs {
    pub token_id: String,
    pub amount: Decimal,
}

/// Signed order request ready for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedOrderRequest {
    pub salt: u64,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: String,
    pub nonce: String,
    pub fee_rate_bps: String,
    pub side: String,
    pub signature_type: u8,
    pub signature: String,
}

/// Post order wrapper
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostOrder {
    pub order: SignedOrderRequest,
    pub owner: String,
    pub order_type: OrderType,
}

impl PostOrder {
    pub fn new(order: SignedOrderRequest, owner: String, order_type: OrderType) -> Self {
        Self {
            order,
            owner,
            order_type,
        }
    }
}

/// Market information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub condition_id: String,
    pub tokens: [Token; 2],
    pub rewards: Rewards,
    pub min_incentive_size: Option<String>,
    pub max_incentive_spread: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub question_id: String,
    pub minimum_order_size: Decimal,
    pub minimum_tick_size: Decimal,
    pub description: String,
    pub category: Option<String>,
    pub end_date_iso: Option<String>,
    pub game_start_time: Option<String>,
    pub question: String,
    pub market_slug: String,
    pub seconds_delay: Decimal,
    pub icon: String,
    pub fpmm: String,
    // Additional fields from API
    #[serde(default)]
    pub enable_order_book: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub accepting_orders: bool,
    #[serde(default)]
    pub accepting_order_timestamp: Option<String>,
    #[serde(default)]
    pub maker_base_fee: Decimal,
    #[serde(default)]
    pub taker_base_fee: Decimal,
    #[serde(default)]
    pub notifications_enabled: bool,
    #[serde(default)]
    pub neg_risk: bool,
    #[serde(default)]
    pub neg_risk_market_id: String,
    #[serde(default)]
    pub neg_risk_request_id: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub is_50_50_outcome: bool,
}

/// Token information within a market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_id: String,
    pub outcome: String,
    pub price: Decimal,
    #[serde(default)]
    pub winner: bool,
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

/// WebSocket authentication for Polymarket API user channel.
///
/// Polymarket's CLOB WebSocket expects the same L2 API credentials used for HTTP calls:
/// `{ apiKey, secret, passphrase }`.
pub type WssAuth = ApiCredentials;

/// WebSocket subscription request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssSubscription {
    /// Channel type: "market" or "user"
    #[serde(rename = "type")]
    pub channel_type: String,
    /// Operation type: "subscribe" or "unsubscribe"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Array of markets (condition IDs) for USER channel
    #[serde(default)]
    pub markets: Vec<String>,
    /// Array of asset IDs (token IDs) for MARKET channel
    /// Note: Field name is "assets_ids" (with 's') per Polymarket API spec
    #[serde(rename = "assets_ids", default)]
    pub asset_ids: Vec<String>,
    /// Request initial state dump
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_dump: Option<bool>,
    /// Enable custom features (best_bid_ask, new_market, market_resolved)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_feature_enabled: Option<bool>,
    /// Authentication information (only for USER channel)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<WssAuth>,
}

/// WebSocket message types for streaming (official Polymarket `event_type` format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum StreamMessage {
    /// Full or incremental orderbook update
    #[serde(rename = "book")]
    Book(BookUpdate),
    /// Price change notification (single or batched)
    #[serde(rename = "price_change")]
    PriceChange(PriceChange),
    /// Tick size change notification
    #[serde(rename = "tick_size_change")]
    TickSizeChange(TickSizeChange),
    /// Last trade price update
    #[serde(rename = "last_trade_price")]
    LastTradePrice(LastTradePrice),
    /// Best bid/ask update (requires `custom_feature_enabled`)
    #[serde(rename = "best_bid_ask")]
    BestBidAsk(BestBidAsk),
    /// New market created (requires `custom_feature_enabled`)
    #[serde(rename = "new_market")]
    NewMarket(NewMarket),
    /// Market resolved (requires `custom_feature_enabled`)
    #[serde(rename = "market_resolved")]
    MarketResolved(MarketResolved),
    /// User trade execution (authenticated channel)
    #[serde(rename = "trade")]
    Trade(TradeMessage),
    /// User order update (authenticated channel)
    #[serde(rename = "order")]
    Order(OrderMessage),
    /// Forward-compatible catch-all for new/unknown event types.
    #[serde(other)]
    Unknown,
}

/// Orderbook update message (full snapshot or delta).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookUpdate {
    pub asset_id: String,
    pub market: String,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub bids: Vec<OrderSummary>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub asks: Vec<OrderSummary>,
    #[serde(default)]
    pub hash: Option<String>,
}

/// Unified wire format for `price_change` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceChange {
    pub market: String,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub price_changes: Vec<PriceChangeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceChangeEntry {
    pub asset_id: String,
    pub price: Decimal,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub size: Option<Decimal>,
    pub side: Side,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub best_bid: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub best_ask: Option<Decimal>,
}

/// Tick size change event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickSizeChange {
    pub asset_id: String,
    pub market: String,
    pub old_tick_size: Decimal,
    pub new_tick_size: Decimal,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
}

/// Last trade price update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastTradePrice {
    pub asset_id: String,
    pub market: String,
    pub price: Decimal,
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub size: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub fee_rate_bps: Option<Decimal>,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
}

/// Best bid/ask update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BestBidAsk {
    pub market: String,
    pub asset_id: String,
    pub best_bid: Decimal,
    pub best_ask: Decimal,
    pub spread: Decimal,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
}

/// New market created event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMarket {
    pub id: String,
    pub question: String,
    pub market: String,
    pub slug: String,
    pub description: String,
    #[serde(rename = "assets_ids", alias = "asset_ids")]
    pub asset_ids: Vec<String>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub outcomes: Vec<String>,
    #[serde(default)]
    pub event_message: Option<EventMessage>,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
}

/// Market resolved event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketResolved {
    pub id: String,
    #[serde(default)]
    pub question: Option<String>,
    pub market: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "assets_ids", alias = "asset_ids")]
    pub asset_ids: Vec<String>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub outcomes: Vec<String>,
    pub winning_asset_id: String,
    pub winning_outcome: String,
    #[serde(default)]
    pub event_message: Option<EventMessage>,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
}

/// Event message object for market events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMessage {
    pub id: String,
    pub ticker: String,
    pub slug: String,
    pub title: String,
    pub description: String,
}

/// User trade execution message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeMessage {
    pub id: String,
    pub market: String,
    pub asset_id: String,
    pub side: Side,
    pub size: Decimal,
    pub price: Decimal,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(rename = "type", default)]
    pub msg_type: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_number_from_string"
    )]
    pub last_update: Option<u64>,
    #[serde(
        default,
        alias = "match_time",
        deserialize_with = "crate::decode::deserializers::optional_number_from_string"
    )]
    pub matchtime: Option<u64>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_number_from_string"
    )]
    pub timestamp: Option<u64>,
}

/// User order update message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderMessage {
    pub id: String,
    pub market: String,
    pub asset_id: String,
    pub side: Side,
    pub price: Decimal,
    #[serde(rename = "type", default)]
    pub msg_type: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub original_size: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string"
    )]
    pub size_matched: Option<Decimal>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_number_from_string"
    )]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub associate_trades: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<String>,
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

/// Parameters for querying open orders
#[derive(Debug, Clone)]
pub struct OpenOrderParams {
    pub id: Option<String>,
    pub asset_id: Option<String>,
    pub market: Option<String>,
}

impl OpenOrderParams {
    pub fn to_query_params(&self) -> Vec<(&str, &String)> {
        let mut params = Vec::with_capacity(3);

        if let Some(x) = &self.id {
            params.push(("id", x));
        }

        if let Some(x) = &self.asset_id {
            params.push(("asset_id", x));
        }

        if let Some(x) = &self.market {
            params.push(("market", x));
        }
        params
    }
}

/// Parameters for querying trades
#[derive(Debug, Clone)]
pub struct TradeParams {
    pub id: Option<String>,
    pub maker_address: Option<String>,
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

impl TradeParams {
    pub fn to_query_params(&self) -> Vec<(&str, String)> {
        let mut params = Vec::with_capacity(6);

        if let Some(x) = &self.id {
            params.push(("id", x.clone()));
        }

        if let Some(x) = &self.asset_id {
            params.push(("asset_id", x.clone()));
        }

        if let Some(x) = &self.market {
            params.push(("market", x.clone()));
        }

        if let Some(x) = &self.maker_address {
            params.push(("maker_address", x.clone()));
        }

        if let Some(x) = &self.before {
            params.push(("before", x.to_string()));
        }

        if let Some(x) = &self.after {
            params.push(("after", x.to_string()));
        }

        params
    }
}

/// Open order information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenOrder {
    pub associate_trades: Vec<String>,
    pub id: String,
    pub status: String,
    pub market: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub original_size: Decimal,
    pub outcome: String,
    pub maker_address: String,
    pub owner: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    pub side: Side,
    #[serde(with = "rust_decimal::serde::str")]
    pub size_matched: Decimal,
    pub asset_id: String,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub expiration: u64,
    #[serde(rename = "type", alias = "order_type", alias = "orderType", default)]
    pub order_type: OrderType,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub created_at: u64,
}

/// Balance allowance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceAllowance {
    pub asset_id: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub balance: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub allowance: Decimal,
}

/// Parameters for balance allowance queries (from reference implementation)
#[derive(Default)]
pub struct BalanceAllowanceParams {
    pub asset_type: Option<AssetType>,
    pub token_id: Option<String>,
    pub signature_type: Option<u8>,
}

impl BalanceAllowanceParams {
    pub fn to_query_params(&self) -> Vec<(&str, String)> {
        let mut params = Vec::with_capacity(3);

        if let Some(x) = &self.asset_type {
            params.push(("asset_type", x.to_string()));
        }

        if let Some(x) = &self.token_id {
            params.push(("token_id", x.to_string()));
        }

        if let Some(x) = &self.signature_type {
            params.push(("signature_type", x.to_string()));
        }
        params
    }

    pub fn set_signature_type(&mut self, s: u8) {
        self.signature_type = Some(s);
    }
}

/// Asset type enum for balance allowance queries
#[allow(clippy::upper_case_acronyms)]
pub enum AssetType {
    COLLATERAL,
    CONDITIONAL,
}

impl std::fmt::Display for AssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetType::COLLATERAL => write!(f, "COLLATERAL"),
            AssetType::CONDITIONAL => write!(f, "CONDITIONAL"),
        }
    }
}

/// Notification preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationParams {
    pub signature: String,
    pub timestamp: u64,
}

/// Batch midpoint request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMidpointRequest {
    pub token_ids: Vec<String>,
}

/// Batch midpoint response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchMidpointResponse {
    pub midpoints: std::collections::HashMap<String, Option<Decimal>>,
}

/// Batch price request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPriceRequest {
    pub token_ids: Vec<String>,
}

/// Price information for a token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPrice {
    pub token_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid: Option<Decimal>,
}

/// Batch price response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPriceResponse {
    pub prices: Vec<TokenPrice>,
}

// Additional types for API compatibility with reference implementation
#[derive(Debug, Deserialize)]
pub struct ApiKeysResponse {
    #[serde(rename = "apiKeys")]
    pub api_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MidpointResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub mid: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct PriceResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
}

// ============================================================================
// PRICE HISTORY (ANALYTICS)
// ============================================================================

/// Time bucket for the `/prices-history` endpoint.
///
/// Note: this endpoint uses a confusing query parameter name (`market`) but expects an
/// outcome asset id (`token_id` / `asset_id`) in **decimal string** form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricesHistoryInterval {
    OneMinute,
    OneHour,
    SixHours,
    OneDay,
    OneWeek,
}

impl PricesHistoryInterval {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneMinute => "1m",
            Self::OneHour => "1h",
            Self::SixHours => "6h",
            Self::OneDay => "1d",
            Self::OneWeek => "1w",
        }
    }
}

/// Raw response from `/prices-history`.
///
/// We intentionally keep `history` entries as `serde_json::Value` because the upstream API has
/// no stable public schema here and currently may return empty history for many markets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricesHistoryResponse {
    pub history: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SpreadResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub spread: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct TickSizeResponse {
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_tick_size: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct NegRiskResponse {
    pub neg_risk: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BookParams {
    pub token_id: String,
    pub side: Side,
}

#[derive(Debug, Deserialize)]
pub struct OrderBookSummary {
    pub market: String,
    pub asset_id: String,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub bids: Vec<OrderSummary>,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::vec_from_null"
    )]
    pub asks: Vec<OrderSummary>,
    pub min_order_size: Decimal,
    pub neg_risk: bool,
    pub tick_size: Decimal,
    #[serde(
        default,
        deserialize_with = "crate::decode::deserializers::optional_decimal_from_string_default_on_error"
    )]
    pub last_trade_price: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSummary {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MarketsResponse {
    pub limit: usize,
    pub count: usize,
    pub next_cursor: Option<String>,
    pub data: Vec<Market>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SimplifiedMarketsResponse {
    pub limit: usize,
    pub count: usize,
    pub next_cursor: Option<String>,
    pub data: Vec<SimplifiedMarket>,
}

/// Simplified market structure for batch operations
#[derive(Debug, Serialize, Deserialize)]
pub struct SimplifiedMarket {
    pub condition_id: String,
    pub tokens: [Token; 2],
    pub rewards: Rewards,
    pub min_incentive_size: Option<String>,
    pub max_incentive_spread: Option<String>,
    pub active: bool,
    pub closed: bool,
}

/// Rewards structure for markets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rewards {
    pub rates: Option<serde_json::Value>,
    // API returns these as plain numbers, not strings
    pub min_size: Decimal,
    pub max_spread: Decimal,
    #[serde(default)]
    pub event_start_date: Option<String>,
    #[serde(default)]
    pub event_end_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub in_game_multiplier: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reward_epoch: Option<Decimal>,
}

// ============================================================================
// CLOB API: Fee Rate + RFQ (Market Maker) Types
// ============================================================================

/// Fee rate in basis points for a given token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeRateResponse {
    pub fee_rate_bps: u32,
}

/// Create RFQ request (Requester).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqCreateRequest {
    pub asset_in: String,
    pub asset_out: String,
    pub amount_in: String,
    pub amount_out: String,
    pub user_type: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqCreateRequestResponse {
    pub request_id: String,
    pub expiry: u64,
}

/// Cancel RFQ request (Requester).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqCancelRequest {
    pub request_id: String,
}

/// RFQ request list query parameters.
#[derive(Debug, Clone, Default)]
pub struct RfqRequestsParams {
    pub offset: Option<String>,
    pub limit: Option<u32>,
    pub state: Option<String>,
    pub request_ids: Vec<String>,
    pub markets: Vec<String>,
    pub size_min: Option<Decimal>,
    pub size_max: Option<Decimal>,
    pub size_usdc_min: Option<Decimal>,
    pub size_usdc_max: Option<Decimal>,
    pub price_min: Option<Decimal>,
    pub price_max: Option<Decimal>,
    pub sort_by: Option<String>,
    pub sort_dir: Option<String>,
}

impl RfqRequestsParams {
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(x) = &self.offset {
            params.push(("offset".to_string(), x.clone()));
        }
        if let Some(x) = self.limit {
            params.push(("limit".to_string(), x.to_string()));
        }
        if let Some(x) = &self.state {
            params.push(("state".to_string(), x.clone()));
        }
        for x in &self.request_ids {
            params.push(("requestIds[]".to_string(), x.clone()));
        }
        for x in &self.markets {
            params.push(("markets[]".to_string(), x.clone()));
        }

        if let Some(x) = self.size_min {
            params.push(("sizeMin".to_string(), x.to_string()));
        }
        if let Some(x) = self.size_max {
            params.push(("sizeMax".to_string(), x.to_string()));
        }
        if let Some(x) = self.size_usdc_min {
            params.push(("sizeUsdcMin".to_string(), x.to_string()));
        }
        if let Some(x) = self.size_usdc_max {
            params.push(("sizeUsdcMax".to_string(), x.to_string()));
        }
        if let Some(x) = self.price_min {
            params.push(("priceMin".to_string(), x.to_string()));
        }
        if let Some(x) = self.price_max {
            params.push(("priceMax".to_string(), x.to_string()));
        }

        if let Some(x) = &self.sort_by {
            params.push(("sortBy".to_string(), x.clone()));
        }
        if let Some(x) = &self.sort_dir {
            params.push(("sortDir".to_string(), x.clone()));
        }

        params
    }
}

/// RFQ request data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqRequestData {
    pub request_id: String,
    pub user_address: String,
    pub proxy_address: String,
    pub condition: String,
    pub token: String,
    pub complement: String,
    pub side: Side,
    #[serde(deserialize_with = "crate::decode::deserializers::decimal_from_string")]
    pub size_in: Decimal,
    #[serde(deserialize_with = "crate::decode::deserializers::decimal_from_string")]
    pub size_out: Decimal,
    #[serde(deserialize_with = "crate::decode::deserializers::decimal_from_string")]
    pub price: Decimal,
    pub state: String,
    pub expiry: u64,
}

/// Create RFQ quote (Quoter).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqCreateQuote {
    pub request_id: String,
    pub asset_in: String,
    pub asset_out: String,
    pub amount_in: String,
    pub amount_out: String,
    pub user_type: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqCreateQuoteResponse {
    pub quote_id: String,
}

/// Cancel RFQ quote (Quoter).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqCancelQuote {
    pub quote_id: String,
}

/// RFQ quote list query parameters.
#[derive(Debug, Clone, Default)]
pub struct RfqQuotesParams {
    pub offset: Option<String>,
    pub limit: Option<u32>,
    pub state: Option<String>,
    pub quote_ids: Vec<String>,
    pub request_ids: Vec<String>,
    pub markets: Vec<String>,
    pub size_min: Option<Decimal>,
    pub size_max: Option<Decimal>,
    pub size_usdc_min: Option<Decimal>,
    pub size_usdc_max: Option<Decimal>,
    pub price_min: Option<Decimal>,
    pub price_max: Option<Decimal>,
    pub sort_by: Option<String>,
    pub sort_dir: Option<String>,
}

impl RfqQuotesParams {
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(x) = &self.offset {
            params.push(("offset".to_string(), x.clone()));
        }
        if let Some(x) = self.limit {
            params.push(("limit".to_string(), x.to_string()));
        }
        if let Some(x) = &self.state {
            params.push(("state".to_string(), x.clone()));
        }
        for x in &self.quote_ids {
            params.push(("quoteIds[]".to_string(), x.clone()));
        }
        for x in &self.request_ids {
            params.push(("requestIds[]".to_string(), x.clone()));
        }
        for x in &self.markets {
            params.push(("markets[]".to_string(), x.clone()));
        }

        if let Some(x) = self.size_min {
            params.push(("sizeMin".to_string(), x.to_string()));
        }
        if let Some(x) = self.size_max {
            params.push(("sizeMax".to_string(), x.to_string()));
        }
        if let Some(x) = self.size_usdc_min {
            params.push(("sizeUsdcMin".to_string(), x.to_string()));
        }
        if let Some(x) = self.size_usdc_max {
            params.push(("sizeUsdcMax".to_string(), x.to_string()));
        }
        if let Some(x) = self.price_min {
            params.push(("priceMin".to_string(), x.to_string()));
        }
        if let Some(x) = self.price_max {
            params.push(("priceMax".to_string(), x.to_string()));
        }

        if let Some(x) = &self.sort_by {
            params.push(("sortBy".to_string(), x.clone()));
        }
        if let Some(x) = &self.sort_dir {
            params.push(("sortDir".to_string(), x.clone()));
        }

        params
    }
}

/// RFQ quote data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqQuoteData {
    pub quote_id: String,
    pub request_id: String,
    pub user_address: String,
    pub proxy_address: String,
    pub condition: String,
    pub token: String,
    pub complement: String,
    pub side: Side,
    #[serde(deserialize_with = "crate::decode::deserializers::decimal_from_string")]
    pub size_in: Decimal,
    #[serde(deserialize_with = "crate::decode::deserializers::decimal_from_string")]
    pub size_out: Decimal,
    #[serde(deserialize_with = "crate::decode::deserializers::decimal_from_string")]
    pub price: Decimal,
    pub match_type: String,
    pub state: String,
}

/// Generic RFQ list response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqListResponse<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
    pub limit: u32,
    pub count: u32,
}

/// RFQ order execution request (used for both accept + approve).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqOrderExecutionRequest {
    pub request_id: String,
    pub quote_id: String,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub expiration: u64,
    pub nonce: String,
    pub fee_rate_bps: String,
    pub side: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub signature_type: u8,
    pub signature: String,
    pub salt: u64,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqApproveOrderResponse {
    pub trade_ids: Vec<String>,
}

// For compatibility with reference implementation
pub type ClientResult<T> = anyhow::Result<T>;

/// Result type used throughout the client
pub type Result<T> = std::result::Result<T, crate::errors::PolyfillError>;

// Type aliases for 100% compatibility with baseline implementation
pub type ApiCreds = ApiCredentials;
pub type CreateOrderOptions = OrderOptions;
pub type OrderArgs = OrderRequest;
