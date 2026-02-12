//! Trade execution and fill handling for Polymarket client
//!
//! This module provides high-performance trade execution logic and
//! fill event processing for latency-sensitive trading environments.

use crate::errors::{PolyfillError, Result};
use crate::types::*;
use crate::utils::math;
use alloy_primitives::Address;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Fill execution result
#[derive(Debug, Clone)]
pub struct FillResult {
    pub order_id: String,
    pub fills: Vec<FillEvent>,
    pub total_size: Decimal,
    pub average_price: Decimal,
    pub total_cost: Decimal,
    pub fees: Decimal,
    pub status: FillStatus,
    pub timestamp: DateTime<Utc>,
}

/// Fill execution status
#[derive(Debug, Clone, PartialEq)]
pub enum FillStatus {
    /// Order was fully filled
    Filled,
    /// Order was partially filled
    Partial,
    /// Order was not filled (insufficient liquidity)
    Unfilled,
    /// Order was rejected
    Rejected,
}

/// Fill execution engine
#[derive(Debug)]
pub struct FillEngine {
    /// Minimum fill size for market orders
    min_fill_size: Decimal,
    /// Maximum slippage tolerance (as percentage)
    max_slippage_pct: Decimal,
    /// Fee rate in basis points
    fee_rate_bps: u32,
    /// Track fills by order ID
    fills: HashMap<String, Vec<FillEvent>>,
}

impl FillEngine {
    /// Create a new fill engine
    pub fn new(min_fill_size: Decimal, max_slippage_pct: Decimal, fee_rate_bps: u32) -> Self {
        Self {
            min_fill_size,
            max_slippage_pct,
            fee_rate_bps,
            fills: HashMap::new(),
        }
    }

    /// Execute a market order against an order book
    pub fn execute_market_order(
        &mut self,
        order: &MarketOrderRequest,
        book: &crate::book::OrderBook,
    ) -> Result<FillResult> {
        let start_time = Utc::now();

        // Validate order
        self.validate_market_order(order)?;

        // Get available liquidity
        let levels = match order.side {
            Side::BUY => book.asks(None),
            Side::SELL => book.bids(None),
        };

        if levels.is_empty() {
            return Ok(FillResult {
                order_id: order
                    .client_id
                    .clone()
                    .unwrap_or_else(|| "market_order".to_string()),
                fills: Vec::new(),
                total_size: Decimal::ZERO,
                average_price: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                fees: Decimal::ZERO,
                status: FillStatus::Unfilled,
                timestamp: start_time,
            });
        }

        // Execute fills
        let mut fills = Vec::new();
        let mut remaining_size = order.amount;
        let mut total_cost = Decimal::ZERO;
        let mut total_size = Decimal::ZERO;

        for level in levels {
            if remaining_size.is_zero() {
                break;
            }

            let fill_size = std::cmp::min(remaining_size, level.size);
            let fill_cost = fill_size * level.price;

            // Calculate fee
            let fee = self.calculate_fee(fill_cost);

            let fill = FillEvent {
                id: uuid::Uuid::new_v4().to_string(),
                order_id: order
                    .client_id
                    .clone()
                    .unwrap_or_else(|| "market_order".to_string()),
                token_id: order.token_id.clone(),
                side: order.side,
                price: level.price,
                size: fill_size,
                timestamp: Utc::now(),
                maker_address: Address::ZERO, // TODO: Get from level
                taker_address: Address::ZERO, // TODO: Get from order
                fee,
            };

            fills.push(fill);
            total_cost += fill_cost;
            total_size += fill_size;
            remaining_size -= fill_size;
        }

        // Check slippage
        if let Some(slippage) = self.calculate_slippage(order, &fills) {
            if slippage > self.max_slippage_pct {
                warn!(
                    "Slippage {}% exceeds maximum {}%",
                    slippage, self.max_slippage_pct
                );
                return Ok(FillResult {
                    order_id: order
                        .client_id
                        .clone()
                        .unwrap_or_else(|| "market_order".to_string()),
                    fills: Vec::new(),
                    total_size: Decimal::ZERO,
                    average_price: Decimal::ZERO,
                    total_cost: Decimal::ZERO,
                    fees: Decimal::ZERO,
                    status: FillStatus::Rejected,
                    timestamp: start_time,
                });
            }
        }

        // Determine status
        let status = if remaining_size.is_zero() {
            FillStatus::Filled
        } else if total_size >= self.min_fill_size {
            FillStatus::Partial
        } else {
            FillStatus::Unfilled
        };

        let average_price = if total_size.is_zero() {
            Decimal::ZERO
        } else {
            total_cost / total_size
        };

        let total_fees: Decimal = fills.iter().map(|f| f.fee).sum();

        let result = FillResult {
            order_id: order
                .client_id
                .clone()
                .unwrap_or_else(|| "market_order".to_string()),
            fills,
            total_size,
            average_price,
            total_cost,
            fees: total_fees,
            status,
            timestamp: start_time,
        };

        // Store fills for tracking
        if !result.fills.is_empty() {
            self.fills
                .insert(result.order_id.clone(), result.fills.clone());
        }

        info!(
            "Market order executed: {} {} @ {} (avg: {})",
            result.total_size,
            order.side.as_str(),
            order.amount,
            result.average_price
        );

        Ok(result)
    }

    /// Execute a limit order (simulation)
    pub fn execute_limit_order(
        &mut self,
        order: &OrderRequest,
        book: &crate::book::OrderBook,
    ) -> Result<FillResult> {
        let start_time = Utc::now();

        // Validate order
        self.validate_limit_order(order)?;

        // Check if order can be filled immediately
        let can_fill = match order.side {
            Side::BUY => {
                if let Some(best_ask) = book.best_ask() {
                    order.price >= best_ask.price
                } else {
                    false
                }
            },
            Side::SELL => {
                if let Some(best_bid) = book.best_bid() {
                    order.price <= best_bid.price
                } else {
                    false
                }
            },
        };

        if !can_fill {
            return Ok(FillResult {
                order_id: order
                    .client_id
                    .clone()
                    .unwrap_or_else(|| "limit_order".to_string()),
                fills: Vec::new(),
                total_size: Decimal::ZERO,
                average_price: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                fees: Decimal::ZERO,
                status: FillStatus::Unfilled,
                timestamp: start_time,
            });
        }

        // Simulate immediate fill
        let fill = FillEvent {
            id: uuid::Uuid::new_v4().to_string(),
            order_id: order
                .client_id
                .clone()
                .unwrap_or_else(|| "limit_order".to_string()),
            token_id: order.token_id.clone(),
            side: order.side,
            price: order.price,
            size: order.size,
            timestamp: Utc::now(),
            maker_address: Address::ZERO,
            taker_address: Address::ZERO,
            fee: self.calculate_fee(order.price * order.size),
        };

        let result = FillResult {
            order_id: order
                .client_id
                .clone()
                .unwrap_or_else(|| "limit_order".to_string()),
            fills: vec![fill],
            total_size: order.size,
            average_price: order.price,
            total_cost: order.price * order.size,
            fees: self.calculate_fee(order.price * order.size),
            status: FillStatus::Filled,
            timestamp: start_time,
        };

        // Store fills for tracking
        self.fills
            .insert(result.order_id.clone(), result.fills.clone());

        info!(
            "Limit order executed: {} {} @ {}",
            result.total_size,
            order.side.as_str(),
            result.average_price
        );

        Ok(result)
    }

    /// Calculate slippage for a market order
    fn calculate_slippage(
        &self,
        order: &MarketOrderRequest,
        fills: &[FillEvent],
    ) -> Option<Decimal> {
        if fills.is_empty() {
            return None;
        }

        let total_cost: Decimal = fills.iter().map(|f| f.price * f.size).sum();
        let total_size: Decimal = fills.iter().map(|f| f.size).sum();
        let average_price = total_cost / total_size;

        // Get reference price (best bid/ask)
        let reference_price = match order.side {
            Side::BUY => fills.first()?.price,  // Best ask
            Side::SELL => fills.first()?.price, // Best bid
        };

        Some(math::calculate_slippage(
            reference_price,
            average_price,
            order.side,
        ))
    }

    /// Calculate fee for a trade
    fn calculate_fee(&self, notional: Decimal) -> Decimal {
        notional * Decimal::from(self.fee_rate_bps) / Decimal::from(10_000)
    }

    /// Validate market order parameters
    fn validate_market_order(&self, order: &MarketOrderRequest) -> Result<()> {
        if order.amount.is_zero() {
            return Err(PolyfillError::order(
                "Market order amount cannot be zero",
                crate::errors::OrderErrorKind::InvalidSize,
            ));
        }

        if order.amount < self.min_fill_size {
            return Err(PolyfillError::order(
                format!(
                    "Order size {} below minimum {}",
                    order.amount, self.min_fill_size
                ),
                crate::errors::OrderErrorKind::SizeConstraint,
            ));
        }

        Ok(())
    }

    /// Validate limit order parameters
    fn validate_limit_order(&self, order: &OrderRequest) -> Result<()> {
        if order.size.is_zero() {
            return Err(PolyfillError::order(
                "Limit order size cannot be zero",
                crate::errors::OrderErrorKind::InvalidSize,
            ));
        }

        if order.price.is_zero() {
            return Err(PolyfillError::order(
                "Limit order price cannot be zero",
                crate::errors::OrderErrorKind::InvalidPrice,
            ));
        }

        if order.size < self.min_fill_size {
            return Err(PolyfillError::order(
                format!(
                    "Order size {} below minimum {}",
                    order.size, self.min_fill_size
                ),
                crate::errors::OrderErrorKind::SizeConstraint,
            ));
        }

        Ok(())
    }

    /// Get fills for an order
    pub fn get_fills(&self, order_id: &str) -> Option<&[FillEvent]> {
        self.fills.get(order_id).map(|f| f.as_slice())
    }

    /// Get all fills
    pub fn get_all_fills(&self) -> Vec<&FillEvent> {
        self.fills.values().flatten().collect()
    }

    /// Clear fills for an order
    pub fn clear_fills(&mut self, order_id: &str) {
        self.fills.remove(order_id);
    }

    /// Get fill statistics
    pub fn get_stats(&self) -> FillStats {
        let total_fills = self.fills.values().flatten().count();
        let total_volume: Decimal = self.fills.values().flatten().map(|f| f.size).sum();
        let total_fees: Decimal = self.fills.values().flatten().map(|f| f.fee).sum();

        FillStats {
            total_orders: self.fills.len(),
            total_fills,
            total_volume,
            total_fees,
        }
    }
}

/// Fill statistics
#[derive(Debug, Clone)]
pub struct FillStats {
    pub total_orders: usize,
    pub total_fills: usize,
    pub total_volume: Decimal,
    pub total_fees: Decimal,
}

/// Fill event processor for real-time updates
#[derive(Debug)]
pub struct FillProcessor {
    /// Pending fills by order ID
    pending_fills: HashMap<String, Vec<FillEvent>>,
    /// Processed fills
    processed_fills: Vec<FillEvent>,
    /// Maximum pending fills to keep in memory
    max_pending: usize,
}

impl FillProcessor {
    /// Create a new fill processor
    pub fn new(max_pending: usize) -> Self {
        Self {
            pending_fills: HashMap::new(),
            processed_fills: Vec::new(),
            max_pending,
        }
    }

    /// Process a fill event
    pub fn process_fill(&mut self, fill: FillEvent) -> Result<()> {
        // Validate fill
        self.validate_fill(&fill)?;

        // Add to pending fills
        self.pending_fills
            .entry(fill.order_id.clone())
            .or_default()
            .push(fill.clone());

        // Move to processed if complete
        if self.is_order_complete(&fill.order_id) {
            if let Some(fills) = self.pending_fills.remove(&fill.order_id) {
                self.processed_fills.extend(fills);
            }
        }

        // Cleanup if too many pending
        if self.pending_fills.len() > self.max_pending {
            self.cleanup_old_pending();
        }

        debug!(
            "Processed fill: {} {} @ {}",
            fill.size,
            fill.side.as_str(),
            fill.price
        );

        Ok(())
    }

    /// Validate a fill event
    fn validate_fill(&self, fill: &FillEvent) -> Result<()> {
        if fill.size.is_zero() {
            return Err(PolyfillError::order(
                "Fill size cannot be zero",
                crate::errors::OrderErrorKind::InvalidSize,
            ));
        }

        if fill.price.is_zero() {
            return Err(PolyfillError::order(
                "Fill price cannot be zero",
                crate::errors::OrderErrorKind::InvalidPrice,
            ));
        }

        Ok(())
    }

    /// Check if an order is complete
    fn is_order_complete(&self, _order_id: &str) -> bool {
        // Simplified implementation - in practice you'd check against order book
        false
    }

    /// Cleanup old pending fills
    fn cleanup_old_pending(&mut self) {
        // Remove oldest pending fills
        let to_remove = self.pending_fills.len() - self.max_pending;
        let mut keys: Vec<_> = self.pending_fills.keys().cloned().collect();
        keys.sort(); // Simple ordering - in practice you'd use timestamps

        for key in keys.iter().take(to_remove) {
            self.pending_fills.remove(key);
        }
    }

    /// Get pending fills for an order
    pub fn get_pending_fills(&self, order_id: &str) -> Option<&[FillEvent]> {
        self.pending_fills.get(order_id).map(|f| f.as_slice())
    }

    /// Get processed fills
    pub fn get_processed_fills(&self) -> &[FillEvent] {
        &self.processed_fills
    }

    /// Get fill statistics
    pub fn get_stats(&self) -> FillProcessorStats {
        let total_pending: Decimal = self.pending_fills.values().flatten().map(|f| f.size).sum();
        let total_processed: Decimal = self.processed_fills.iter().map(|f| f.size).sum();

        FillProcessorStats {
            pending_orders: self.pending_fills.len(),
            pending_fills: self.pending_fills.values().flatten().count(),
            pending_volume: total_pending,
            processed_fills: self.processed_fills.len(),
            processed_volume: total_processed,
        }
    }
}

/// Fill processor statistics
#[derive(Debug, Clone)]
pub struct FillProcessorStats {
    pub pending_orders: usize,
    pub pending_fills: usize,
    pub pending_volume: Decimal,
    pub processed_fills: usize,
    pub processed_volume: Decimal,
}

// ============================================================================
// QUEUE-BASED FILL SIMULATION ENGINE
// ============================================================================

use crate::types::{
    decimal_to_price, decimal_to_qty, price_to_decimal, qty_to_decimal, Price, Qty,
};

/// State for a single queued order being tracked
#[derive(Debug, Clone)]
pub struct QueuedOrder {
    /// Unique order identifier
    pub order_id: String,
    /// Token this order is for
    pub token_id: String,
    /// Order side
    pub side: Side,
    /// Limit price in fixed-point ticks
    pub price: Price,
    /// Remaining size to fill in fixed-point units
    pub remaining_size: Qty,
    /// Queue position: size ahead of us at our price level
    pub queue_ahead: Qty,
    /// Last observed best bid in ticks
    pub last_best_bid: Option<Price>,
    /// Last observed best ask in ticks
    pub last_best_ask: Option<Price>,
    /// Last observed size at our price level
    pub last_size_at_price: Qty,
    /// Timestamp when order was placed
    pub placed_at: DateTime<Utc>,
}

impl QueuedOrder {
    /// Create a new queued order from decimal inputs
    pub fn new(
        order_id: String,
        token_id: String,
        side: Side,
        price: Decimal,
        size: Decimal,
        initial_queue_ahead: Decimal,
    ) -> Result<Self> {
        let price_ticks = decimal_to_price(price)
            .map_err(|e| PolyfillError::validation(format!("Invalid price: {}", e)))?;
        let size_units = decimal_to_qty(size)
            .map_err(|e| PolyfillError::validation(format!("Invalid size: {}", e)))?;
        let queue_ahead_units = decimal_to_qty(initial_queue_ahead)
            .map_err(|e| PolyfillError::validation(format!("Invalid queue_ahead: {}", e)))?;

        Ok(Self {
            order_id,
            token_id,
            side,
            price: price_ticks,
            remaining_size: size_units,
            queue_ahead: queue_ahead_units,
            last_best_bid: None,
            last_best_ask: None,
            last_size_at_price: 0,
            placed_at: Utc::now(),
        })
    }

    /// Get the limit price as Decimal
    pub fn price_decimal(&self) -> Decimal {
        price_to_decimal(self.price)
    }

    /// Get the remaining size as Decimal
    pub fn remaining_size_decimal(&self) -> Decimal {
        qty_to_decimal(self.remaining_size)
    }

    /// Get the queue ahead as Decimal
    pub fn queue_ahead_decimal(&self) -> Decimal {
        qty_to_decimal(self.queue_ahead)
    }
}

/// Book update information for queue simulation
#[derive(Debug, Clone)]
pub struct BookUpdate {
    /// Token ID
    pub token_id: String,
    /// Current best bid price in ticks
    pub best_bid: Option<Price>,
    /// Current best ask price in ticks
    pub best_ask: Option<Price>,
    /// Current size at specific price levels (price -> size)
    pub sizes_at_prices: Vec<(Price, Qty)>,
    /// Update timestamp
    pub timestamp: DateTime<Utc>,
}

impl BookUpdate {
    /// Create a BookUpdate from an OrderBook
    pub fn from_order_book(book: &crate::book::OrderBook) -> Self {
        let mut sizes_at_prices = Vec::new();

        // Collect bid levels
        for level in book.bids_fast(None) {
            sizes_at_prices.push((level.price, level.size));
        }
        // Collect ask levels
        for level in book.asks_fast(None) {
            sizes_at_prices.push((level.price, level.size));
        }

        Self {
            token_id: book.token_id.clone(),
            best_bid: book.best_bid_fast().map(|l| l.price),
            best_ask: book.best_ask_fast().map(|l| l.price),
            sizes_at_prices,
            timestamp: book.timestamp,
        }
    }

    /// Get size at a specific price
    pub fn size_at_price(&self, price: Price) -> Qty {
        self.sizes_at_prices
            .iter()
            .find(|(p, _)| *p == price)
            .map(|(_, s)| *s)
            .unwrap_or(0)
    }
}

/// Queue-based fill simulation engine
///
/// Simulates fills for resting limit orders using a conservative queue model:
/// - Queue position decreases when size at your price level decreases
/// - Fills only occur when the market trades through your limit price
///
/// This is useful for backtesting and simulation where you want realistic
/// fill assumptions without optimistic "instant fill" behavior.
#[derive(Debug)]
pub struct QueueFillEngine {
    /// Active orders being tracked (order_id -> QueuedOrder)
    orders: HashMap<String, QueuedOrder>,
    /// Completed fills
    fills: Vec<FillEvent>,
}

impl QueueFillEngine {
    /// Create a new queue fill engine
    pub fn new() -> Self {
        Self {
            orders: HashMap::new(),
            fills: Vec::new(),
        }
    }

    /// Add an order to track
    pub fn add_order(&mut self, order: QueuedOrder) {
        self.orders.insert(order.order_id.clone(), order);
    }

    /// Remove an order from tracking
    pub fn remove_order(&mut self, order_id: &str) -> Option<QueuedOrder> {
        self.orders.remove(order_id)
    }

    /// Get a tracked order
    pub fn get_order(&self, order_id: &str) -> Option<&QueuedOrder> {
        self.orders.get(order_id)
    }

    /// Get all tracked orders
    pub fn get_all_orders(&self) -> Vec<&QueuedOrder> {
        self.orders.values().collect()
    }

    /// Get all fills
    pub fn get_fills(&self) -> &[FillEvent] {
        &self.fills
    }

    /// Clear all fills
    pub fn clear_fills(&mut self) {
        self.fills.clear();
    }

    /// Process a book update and generate fill events
    ///
    /// Conservative fill rule:
    /// 1. Size decreases at your price level reduce queue_ahead
    /// 2. Fills only occur when the market trades THROUGH your limit:
    ///    - BUY: fills when best_ask <= limit_price (someone willing to sell at or below your buy price)
    ///    - SELL: fills when best_bid >= limit_price (someone willing to buy at or above your sell price)
    ///
    /// Returns any fill events generated
    pub fn on_book_update(&mut self, update: &BookUpdate) -> Vec<FillEvent> {
        let mut new_fills = Vec::new();
        let timestamp = update.timestamp;

        // Process each order for this token
        let order_ids: Vec<String> = self
            .orders
            .iter()
            .filter(|(_, o)| o.token_id == update.token_id)
            .map(|(id, _)| id.clone())
            .collect();

        for order_id in order_ids {
            if let Some(order) = self.orders.get_mut(&order_id) {
                // Get current size at order's price level
                let current_size_at_price = update.size_at_price(order.price);

                // Step 1: Update queue position based on size reduction
                if order.last_size_at_price > 0 && current_size_at_price < order.last_size_at_price
                {
                    let size_reduction = order.last_size_at_price - current_size_at_price;
                    // Queue ahead decreases by the amount of size that left
                    order.queue_ahead = (order.queue_ahead - size_reduction).max(0);
                }
                order.last_size_at_price = current_size_at_price;

                // Step 2: Check for trade-through fills
                let trade_through = match order.side {
                    // BUY: fills when best_ask <= limit_price
                    Side::BUY => update.best_ask.map_or(false, |ask| ask <= order.price),
                    // SELL: fills when best_bid >= limit_price
                    Side::SELL => update.best_bid.map_or(false, |bid| bid >= order.price),
                };

                // Step 3: Generate fills if trade-through occurred and queue is clear
                if trade_through && order.queue_ahead == 0 && order.remaining_size > 0 {
                    // Determine fill size (could be partial based on available liquidity)
                    // For simplicity, we fill the entire remaining order when trade-through happens
                    let fill_size = order.remaining_size;

                    let fill = FillEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        order_id: order.order_id.clone(),
                        token_id: order.token_id.clone(),
                        side: order.side,
                        price: price_to_decimal(order.price),
                        size: qty_to_decimal(fill_size),
                        timestamp,
                        maker_address: Address::ZERO,
                        taker_address: Address::ZERO,
                        fee: Decimal::ZERO, // Queue fills typically have maker rebate
                    };

                    new_fills.push(fill.clone());
                    self.fills.push(fill);
                    order.remaining_size = 0;
                }

                // Update last observed best prices
                order.last_best_bid = update.best_bid;
                order.last_best_ask = update.best_ask;
            }
        }

        // Remove fully filled orders
        self.orders.retain(|_, o| o.remaining_size > 0);

        new_fills
    }

    /// Get statistics about tracked orders
    pub fn get_stats(&self) -> QueueFillStats {
        let total_orders = self.orders.len();
        let total_fills = self.fills.len();
        let pending_volume: Decimal = self
            .orders
            .values()
            .map(|o| qty_to_decimal(o.remaining_size))
            .sum();
        let filled_volume: Decimal = self.fills.iter().map(|f| f.size).sum();

        QueueFillStats {
            total_orders,
            total_fills,
            pending_volume,
            filled_volume,
        }
    }
}

impl Default for QueueFillEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for queue fill engine
#[derive(Debug, Clone)]
pub struct QueueFillStats {
    pub total_orders: usize,
    pub total_fills: usize,
    pub pending_volume: Decimal,
    pub filled_volume: Decimal,
}

impl FillEngine {
    /// Execute a crossing (marketable) limit order against an order book
    ///
    /// Walks through price levels up to the limit price, simulating fills
    /// at each level. Fee is set to 0 for crossing limit orders.
    ///
    /// A limit order is "crossing" or "marketable" when:
    /// - BUY: limit_price >= best_ask (willing to pay at or above the ask)
    /// - SELL: limit_price <= best_bid (willing to sell at or below the bid)
    pub fn execute_crossing_limit(
        &mut self,
        order: &OrderRequest,
        book: &crate::book::OrderBook,
    ) -> Result<FillResult> {
        let start_time = Utc::now();

        // Validate order
        self.validate_limit_order(order)?;

        // Check if order is marketable (crossing)
        let is_crossing = match order.side {
            Side::BUY => book
                .best_ask()
                .map_or(false, |ask| order.price >= ask.price),
            Side::SELL => book
                .best_bid()
                .map_or(false, |bid| order.price <= bid.price),
        };

        if !is_crossing {
            // Not a crossing order - would rest on book
            return Ok(FillResult {
                order_id: order
                    .client_id
                    .clone()
                    .unwrap_or_else(|| "crossing_limit".to_string()),
                fills: Vec::new(),
                total_size: Decimal::ZERO,
                average_price: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                fees: Decimal::ZERO,
                status: FillStatus::Unfilled,
                timestamp: start_time,
            });
        }

        // Get levels to fill against (up to limit price)
        let levels = match order.side {
            Side::BUY => {
                // For buy, get asks up to limit price
                book.asks(None)
                    .into_iter()
                    .filter(|level| level.price <= order.price)
                    .collect::<Vec<_>>()
            },
            Side::SELL => {
                // For sell, get bids down to limit price
                book.bids(None)
                    .into_iter()
                    .filter(|level| level.price >= order.price)
                    .collect::<Vec<_>>()
            },
        };

        if levels.is_empty() {
            return Ok(FillResult {
                order_id: order
                    .client_id
                    .clone()
                    .unwrap_or_else(|| "crossing_limit".to_string()),
                fills: Vec::new(),
                total_size: Decimal::ZERO,
                average_price: Decimal::ZERO,
                total_cost: Decimal::ZERO,
                fees: Decimal::ZERO,
                status: FillStatus::Unfilled,
                timestamp: start_time,
            });
        }

        // Execute fills level by level
        let mut fills = Vec::new();
        let mut remaining_size = order.size;
        let mut total_cost = Decimal::ZERO;
        let mut total_size = Decimal::ZERO;

        for level in levels {
            if remaining_size.is_zero() {
                break;
            }

            let fill_size = std::cmp::min(remaining_size, level.size);
            let fill_cost = fill_size * level.price;

            let fill = FillEvent {
                id: uuid::Uuid::new_v4().to_string(),
                order_id: order
                    .client_id
                    .clone()
                    .unwrap_or_else(|| "crossing_limit".to_string()),
                token_id: order.token_id.clone(),
                side: order.side,
                price: level.price,
                size: fill_size,
                timestamp: Utc::now(),
                maker_address: Address::ZERO,
                taker_address: Address::ZERO,
                fee: Decimal::ZERO, // Fee is 0 for crossing limit orders
            };

            fills.push(fill);
            total_cost += fill_cost;
            total_size += fill_size;
            remaining_size -= fill_size;
        }

        // Determine status
        let status = if remaining_size.is_zero() {
            FillStatus::Filled
        } else if total_size >= self.min_fill_size {
            FillStatus::Partial
        } else {
            FillStatus::Unfilled
        };

        let average_price = if total_size.is_zero() {
            Decimal::ZERO
        } else {
            total_cost / total_size
        };

        let result = FillResult {
            order_id: order
                .client_id
                .clone()
                .unwrap_or_else(|| "crossing_limit".to_string()),
            fills,
            total_size,
            average_price,
            total_cost,
            fees: Decimal::ZERO,
            status,
            timestamp: start_time,
        };

        // Store fills for tracking
        if !result.fills.is_empty() {
            self.fills
                .insert(result.order_id.clone(), result.fills.clone());
        }

        info!(
            "Crossing limit order executed: {} {} @ {} (limit: {})",
            result.total_size,
            order.side.as_str(),
            result.average_price,
            order.price
        );

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_fill_engine_creation() {
        let engine = FillEngine::new(dec!(1), dec!(5), 10);
        assert_eq!(engine.min_fill_size, dec!(1));
        assert_eq!(engine.max_slippage_pct, dec!(5));
        assert_eq!(engine.fee_rate_bps, 10);
    }

    #[test]
    fn test_market_order_validation() {
        let engine = FillEngine::new(dec!(1), dec!(5), 10);

        let valid_order = MarketOrderRequest {
            token_id: "test".to_string(),
            side: Side::BUY,
            amount: dec!(100),
            slippage_tolerance: None,
            client_id: None,
        };
        assert!(engine.validate_market_order(&valid_order).is_ok());

        let invalid_order = MarketOrderRequest {
            token_id: "test".to_string(),
            side: Side::BUY,
            amount: dec!(0),
            slippage_tolerance: None,
            client_id: None,
        };
        assert!(engine.validate_market_order(&invalid_order).is_err());
    }

    #[test]
    fn test_fee_calculation() {
        let engine = FillEngine::new(dec!(1), dec!(5), 10);
        let fee = engine.calculate_fee(dec!(1000));
        assert_eq!(fee, dec!(1)); // 10 bps = 0.1% = 1 on 1000
    }

    #[test]
    fn test_fill_processor() {
        let mut processor = FillProcessor::new(100);

        let fill = FillEvent {
            id: "fill1".to_string(),
            order_id: "order1".to_string(),
            token_id: "test".to_string(),
            side: Side::BUY,
            price: dec!(0.5),
            size: dec!(100),
            timestamp: Utc::now(),
            maker_address: Address::ZERO,
            taker_address: Address::ZERO,
            fee: dec!(0.1),
        };

        assert!(processor.process_fill(fill).is_ok());
        assert_eq!(processor.pending_fills.len(), 1);
    }

    #[test]
    fn test_fill_engine_advanced_creation() {
        // Test that we can create a fill engine with parameters
        let _engine = FillEngine::new(dec!(1.0), dec!(0.05), 50); // min_fill_size, max_slippage, fee_rate_bps

        // Test basic properties exist (we can't access private fields directly)
        // But we can test that the engine was created successfully
        // Engine creation successful
    }

    #[test]
    fn test_fill_processor_basic_operations() {
        let mut processor = FillProcessor::new(100); // max_pending

        // Test that we can create a fill event and process it
        let fill_event = FillEvent {
            id: "fill_1".to_string(),
            order_id: "order_1".to_string(),
            side: Side::BUY,
            size: dec!(25),
            price: dec!(0.75),
            timestamp: chrono::Utc::now(),
            token_id: "token_1".to_string(),
            maker_address: alloy_primitives::Address::ZERO,
            taker_address: alloy_primitives::Address::ZERO,
            fee: dec!(0.01),
        };

        let result = processor.process_fill(fill_event);
        assert!(result.is_ok());

        // Check that the fill was added to pending
        assert_eq!(processor.pending_fills.len(), 1);
    }

    // ========================================================================
    // QueueFillEngine tests
    // ========================================================================

    #[test]
    fn test_queue_fill_engine_creation() {
        let engine = QueueFillEngine::new();
        assert_eq!(engine.get_all_orders().len(), 0);
        assert_eq!(engine.get_fills().len(), 0);
    }

    #[test]
    fn test_queue_reduction_without_trade_through_no_fills() {
        // Test: queue_ahead decreases when size at price decreases,
        // but no fills occur because there's no trade-through
        let mut engine = QueueFillEngine::new();

        // Add a BUY order at price 0.50 (5000 ticks) with queue of 100 ahead
        let order = QueuedOrder::new(
            "order1".to_string(),
            "test_token".to_string(),
            Side::BUY,
            dec!(0.50),
            dec!(10),  // order size
            dec!(100), // queue_ahead
        )
        .unwrap();

        // Initialize last_size_at_price
        let mut order = order;
        order.last_size_at_price = 1_500_000; // 150 tokens in queue
        engine.add_order(order);

        // Book update: size at 0.50 reduced from 150 to 100 (50 traded)
        // But best_ask is still above our price (0.55), so no trade-through
        let update = BookUpdate {
            token_id: "test_token".to_string(),
            best_bid: Some(4900),                     // 0.49
            best_ask: Some(5500), // 0.55 - above our buy limit, no trade-through
            sizes_at_prices: vec![(5000, 1_000_000)], // 100 at 0.50
            timestamp: Utc::now(),
        };

        let fills = engine.on_book_update(&update);

        // No fills because no trade-through
        assert!(
            fills.is_empty(),
            "Should have no fills without trade-through"
        );

        // But queue_ahead should have decreased
        let order = engine.get_order("order1").unwrap();
        assert!(
            order.queue_ahead < 1_000_000,
            "Queue ahead should have decreased from 100 to 50"
        );
        assert_eq!(
            order.queue_ahead, 500_000,
            "Queue ahead should be 50 (500,000 units)"
        );
    }

    #[test]
    fn test_trade_through_triggers_fill_at_limit() {
        // Test: when price trades through limit and queue is cleared, fills occur
        let mut engine = QueueFillEngine::new();

        // Add a BUY order at price 0.50 with no queue ahead
        let order = QueuedOrder::new(
            "order1".to_string(),
            "test_token".to_string(),
            Side::BUY,
            dec!(0.50),
            dec!(10), // order size
            dec!(0),  // no queue ahead - at front of queue
        )
        .unwrap();
        engine.add_order(order);

        // Book update: best_ask drops to 0.50 (our limit) - trade-through!
        let update = BookUpdate {
            token_id: "test_token".to_string(),
            best_bid: Some(4900),             // 0.49
            best_ask: Some(5000),             // 0.50 - at our limit, triggers fill
            sizes_at_prices: vec![(5000, 0)], // our level is empty
            timestamp: Utc::now(),
        };

        let fills = engine.on_book_update(&update);

        // Should have one fill
        assert_eq!(fills.len(), 1, "Should have exactly one fill");
        let fill = &fills[0];
        assert_eq!(fill.order_id, "order1");
        assert_eq!(fill.size, dec!(10));
        assert_eq!(fill.price, dec!(0.50));
        assert_eq!(fill.fee, Decimal::ZERO); // Queue fills have zero fee

        // Order should be removed (fully filled)
        assert!(engine.get_order("order1").is_none());
    }

    #[test]
    fn test_sell_order_trade_through() {
        // Test trade-through for SELL orders (best_bid >= limit_price)
        let mut engine = QueueFillEngine::new();

        // Add a SELL order at price 0.60 with no queue ahead
        let order = QueuedOrder::new(
            "sell1".to_string(),
            "test_token".to_string(),
            Side::SELL,
            dec!(0.60),
            dec!(20), // order size
            dec!(0),  // no queue ahead
        )
        .unwrap();
        engine.add_order(order);

        // Book update: best_bid rises to 0.60 (our limit) - trade-through!
        let update = BookUpdate {
            token_id: "test_token".to_string(),
            best_bid: Some(6000), // 0.60 - at our limit, triggers fill
            best_ask: Some(6100), // 0.61
            sizes_at_prices: vec![(6000, 0)],
            timestamp: Utc::now(),
        };

        let fills = engine.on_book_update(&update);

        assert_eq!(fills.len(), 1);
        let fill = &fills[0];
        assert_eq!(fill.order_id, "sell1");
        assert_eq!(fill.side, Side::SELL);
        assert_eq!(fill.size, dec!(20));
        assert_eq!(fill.price, dec!(0.60));
    }

    #[test]
    fn test_no_fill_when_queue_not_cleared() {
        // Test: even with trade-through, no fill if queue_ahead > 0
        let mut engine = QueueFillEngine::new();

        // Add a BUY order with queue ahead
        let order = QueuedOrder::new(
            "order1".to_string(),
            "test_token".to_string(),
            Side::BUY,
            dec!(0.50),
            dec!(10),
            dec!(50), // Still have 50 tokens ahead
        )
        .unwrap();
        engine.add_order(order);

        // Book update: trade-through occurs, but queue not cleared
        let update = BookUpdate {
            token_id: "test_token".to_string(),
            best_bid: Some(4900),
            best_ask: Some(5000),             // Trade-through!
            sizes_at_prices: vec![(5000, 0)], // Level empty but queue_ahead still positive
            timestamp: Utc::now(),
        };

        let fills = engine.on_book_update(&update);

        // No fills because queue_ahead > 0
        assert!(fills.is_empty());
        assert!(engine.get_order("order1").is_some());
    }

    #[test]
    fn test_crossing_limit_order_walks_levels() {
        // Test: crossing limit fills across multiple price levels
        use crate::book::OrderBook;
        use crate::types::OrderType;

        let mut engine = FillEngine::new(dec!(1), dec!(5), 10);
        let mut book = OrderBook::new("test_token".to_string(), 10);

        // Set up ask levels: 100 @ 0.50, 100 @ 0.51, 100 @ 0.52
        book.apply_level_fast(Side::SELL, 5000, 1_000_000); // 100 @ 0.50
        book.apply_level_fast(Side::SELL, 5100, 1_000_000); // 100 @ 0.51
        book.apply_level_fast(Side::SELL, 5200, 1_000_000); // 100 @ 0.52

        // Create a BUY order for 150 @ limit 0.51
        let order = OrderRequest {
            token_id: "test_token".to_string(),
            side: Side::BUY,
            price: dec!(0.51),
            size: dec!(150),
            order_type: OrderType::GTC,
            expiration: None,
            client_id: Some("crossing1".to_string()),
        };

        let result = engine.execute_crossing_limit(&order, &book).unwrap();

        // Should fill 100 @ 0.50 and 50 @ 0.51 (not 0.52 - above limit)
        assert_eq!(result.status, FillStatus::Filled);
        assert_eq!(result.total_size, dec!(150));
        assert_eq!(result.fees, Decimal::ZERO); // Crossing limit has 0 fee
        assert_eq!(result.fills.len(), 2);

        // First fill at 0.50
        assert_eq!(result.fills[0].price, dec!(0.50));
        assert_eq!(result.fills[0].size, dec!(100));

        // Second fill at 0.51
        assert_eq!(result.fills[1].price, dec!(0.51));
        assert_eq!(result.fills[1].size, dec!(50));
    }

    #[test]
    fn test_crossing_limit_partial_fill() {
        // Test: crossing limit gets partial fill when liquidity insufficient
        use crate::book::OrderBook;
        use crate::types::OrderType;

        let mut engine = FillEngine::new(dec!(1), dec!(5), 10);
        let mut book = OrderBook::new("test_token".to_string(), 10);

        // Only 50 tokens available at 0.50
        book.apply_level_fast(Side::SELL, 5000, 500_000); // 50 @ 0.50

        // Create a BUY order for 100 @ limit 0.50
        let order = OrderRequest {
            token_id: "test_token".to_string(),
            side: Side::BUY,
            price: dec!(0.50),
            size: dec!(100),
            order_type: OrderType::GTC,
            expiration: None,
            client_id: Some("partial1".to_string()),
        };

        let result = engine.execute_crossing_limit(&order, &book).unwrap();

        assert_eq!(result.status, FillStatus::Partial);
        assert_eq!(result.total_size, dec!(50));
        assert_eq!(result.fills.len(), 1);
    }

    #[test]
    fn test_non_crossing_limit_returns_unfilled() {
        // Test: non-crossing limit order returns unfilled status
        use crate::book::OrderBook;
        use crate::types::OrderType;

        let mut engine = FillEngine::new(dec!(1), dec!(5), 10);
        let mut book = OrderBook::new("test_token".to_string(), 10);

        // Best ask at 0.55
        book.apply_level_fast(Side::SELL, 5500, 1_000_000); // 100 @ 0.55

        // Create a BUY order for 100 @ limit 0.50 (below best ask)
        let order = OrderRequest {
            token_id: "test_token".to_string(),
            side: Side::BUY,
            price: dec!(0.50),
            size: dec!(100),
            order_type: OrderType::GTC,
            expiration: None,
            client_id: Some("resting1".to_string()),
        };

        let result = engine.execute_crossing_limit(&order, &book).unwrap();

        assert_eq!(result.status, FillStatus::Unfilled);
        assert_eq!(result.total_size, Decimal::ZERO);
        assert!(result.fills.is_empty());
    }

    #[test]
    fn test_queue_fill_stats() {
        let mut engine = QueueFillEngine::new();

        // Add multiple orders
        engine.add_order(
            QueuedOrder::new(
                "order1".to_string(),
                "test".to_string(),
                Side::BUY,
                dec!(0.50),
                dec!(100),
                dec!(0),
            )
            .unwrap(),
        );
        engine.add_order(
            QueuedOrder::new(
                "order2".to_string(),
                "test".to_string(),
                Side::SELL,
                dec!(0.60),
                dec!(50),
                dec!(0),
            )
            .unwrap(),
        );

        let stats = engine.get_stats();
        assert_eq!(stats.total_orders, 2);
        assert_eq!(stats.pending_volume, dec!(150));
    }
}
