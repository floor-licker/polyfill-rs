//! Order book management for Polymarket client
//!
//! This module provides high-performance order book operations optimized
//! for latency-sensitive trading environments.

use crate::errors::{PolyfillError, Result};
use crate::types::*;
use crate::utils::math;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, trace, warn};
use chrono::Utc;
use std::collections::HashMap;

/// High-performance order book implementation
#[derive(Debug, Clone)]
pub struct OrderBook {
    /// Token ID this book represents
    pub token_id: String,
    /// Current sequence number for ordering updates
    pub sequence: u64,
    /// Last update timestamp
    pub timestamp: chrono::DateTime<Utc>,
    /// Bid side (price -> size, sorted descending)
    bids: BTreeMap<Decimal, Decimal>,
    /// Ask side (price -> size, sorted ascending)
    asks: BTreeMap<Decimal, Decimal>,
    /// Minimum tick size for this market
    tick_size: Option<Decimal>,
    /// Maximum depth to maintain
    max_depth: usize,
}

impl OrderBook {
    /// Create a new order book
    pub fn new(token_id: String, max_depth: usize) -> Self {
        Self {
            token_id,
            sequence: 0,
            timestamp: Utc::now(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            tick_size: None,
            max_depth,
        }
    }

    /// Set the tick size for this book
    pub fn set_tick_size(&mut self, tick_size: Decimal) {
        self.tick_size = Some(tick_size);
    }

    /// Get the current best bid
    pub fn best_bid(&self) -> Option<BookLevel> {
        self.bids.iter().next_back().map(|(&price, &size)| BookLevel { price, size })
    }

    /// Get the current best ask
    pub fn best_ask(&self) -> Option<BookLevel> {
        self.asks.iter().next().map(|(&price, &size)| BookLevel { price, size })
    }

    /// Get the current spread
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask.price - bid.price),
            _ => None,
        }
    }

    /// Get the current mid price
    pub fn mid_price(&self) -> Option<Decimal> {
        math::mid_price(
            self.best_bid()?.price,
            self.best_ask()?.price,
        )
    }

    /// Get the spread as a percentage
    pub fn spread_pct(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => math::spread_pct(bid.price, ask.price),
            _ => None,
        }
    }

    /// Get all bids up to a certain depth
    pub fn bids(&self, depth: Option<usize>) -> Vec<BookLevel> {
        let depth = depth.unwrap_or(self.max_depth);
        self.bids
            .iter()
            .rev()
            .take(depth)
            .map(|(&price, &size)| BookLevel { price, size })
            .collect()
    }

    /// Get all asks up to a certain depth
    pub fn asks(&self, depth: Option<usize>) -> Vec<BookLevel> {
        let depth = depth.unwrap_or(self.max_depth);
        self.asks
            .iter()
            .take(depth)
            .map(|(&price, &size)| BookLevel { price, size })
            .collect()
    }

    /// Get the full book snapshot
    pub fn snapshot(&self) -> crate::types::OrderBook {
        crate::types::OrderBook {
            token_id: self.token_id.clone(),
            timestamp: self.timestamp,
            bids: self.bids(None),
            asks: self.asks(None),
            sequence: self.sequence,
        }
    }

    /// Apply a delta update to the book
    pub fn apply_delta(&mut self, delta: OrderDelta) -> Result<()> {
        // Validate sequence ordering
        if delta.sequence <= self.sequence {
            trace!("Ignoring stale delta: {} <= {}", delta.sequence, self.sequence);
            return Ok(());
        }

        // Update sequence and timestamp
        self.sequence = delta.sequence;
        self.timestamp = delta.timestamp;

        // Apply the delta
        match delta.side {
            Side::BUY => self.apply_bid_delta(delta.price, delta.size),
            Side::SELL => self.apply_ask_delta(delta.price, delta.size),
        }

        // Maintain depth limits
        self.trim_depth();

        debug!(
            "Applied delta: {} {} @ {} (seq: {})",
            delta.side.as_str(),
            delta.size,
            delta.price,
            delta.sequence
        );

        Ok(())
    }

    /// Apply a bid-side delta
    fn apply_bid_delta(&mut self, price: Decimal, size: Decimal) {
        if size.is_zero() {
            self.bids.remove(&price);
        } else {
            self.bids.insert(price, size);
        }
    }

    /// Apply an ask-side delta
    fn apply_ask_delta(&mut self, price: Decimal, size: Decimal) {
        if size.is_zero() {
            self.asks.remove(&price);
        } else {
            self.asks.insert(price, size);
        }
    }

    /// Trim the book to maintain depth limits
    fn trim_depth(&mut self) {
        if self.bids.len() > self.max_depth {
            let to_remove = self.bids.len() - self.max_depth;
            for _ in 0..to_remove {
                self.bids.pop_first();
            }
        }

        if self.asks.len() > self.max_depth {
            let to_remove = self.asks.len() - self.max_depth;
            for _ in 0..to_remove {
                self.asks.pop_last();
            }
        }
    }

    /// Calculate the market impact for a given order size
    pub fn calculate_market_impact(&self, side: Side, size: Decimal) -> Option<MarketImpact> {
        let levels = match side {
            Side::BUY => self.asks(None),
            Side::SELL => self.bids(None),
        };

        if levels.is_empty() {
            return None;
        }

        let mut remaining_size = size;
        let mut total_cost = Decimal::ZERO;
        let mut weighted_price = Decimal::ZERO;

        for level in levels {
            let fill_size = std::cmp::min(remaining_size, level.size);
            let level_cost = fill_size * level.price;
            
            total_cost += level_cost;
            weighted_price += level_cost;
            remaining_size -= fill_size;

            if remaining_size.is_zero() {
                break;
            }
        }

        if remaining_size > Decimal::ZERO {
            return None; // Not enough liquidity
        }

        let avg_price = weighted_price / size;
        let impact = match side {
            Side::BUY => {
                let best_ask = self.best_ask()?.price;
                (avg_price - best_ask) / best_ask
            }
            Side::SELL => {
                let best_bid = self.best_bid()?.price;
                (best_bid - avg_price) / best_bid
            }
        };

        Some(MarketImpact {
            average_price: avg_price,
            impact_pct: impact,
            total_cost,
            size_filled: size,
        })
    }

    /// Check if the book is stale (no recent updates)
    pub fn is_stale(&self, max_age: std::time::Duration) -> bool {
        let age = Utc::now() - self.timestamp;
        age > chrono::Duration::from_std(max_age).unwrap_or_default()
    }

    /// Get the total liquidity at a given price level
    pub fn liquidity_at_price(&self, price: Decimal, side: Side) -> Decimal {
        match side {
            Side::BUY => self.asks.get(&price).copied().unwrap_or_default(),
            Side::SELL => self.bids.get(&price).copied().unwrap_or_default(),
        }
    }

    /// Get the total liquidity within a price range
    pub fn liquidity_in_range(&self, min_price: Decimal, max_price: Decimal, side: Side) -> Decimal {
        let levels: Vec<_> = match side {
            Side::BUY => self.asks.range(min_price..=max_price).collect(),
            Side::SELL => self.bids.range(min_price..=max_price).rev().collect(),
        };

        levels.into_iter().map(|(_, &size)| size).sum()
    }

    /// Validate that prices are properly ordered
    pub fn is_valid(&self) -> bool {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => bid.price < ask.price,
            _ => true, // Empty book is valid
        }
    }
}

/// Market impact calculation result
#[derive(Debug, Clone)]
pub struct MarketImpact {
    pub average_price: Decimal,
    pub impact_pct: Decimal,
    pub total_cost: Decimal,
    pub size_filled: Decimal,
}

/// Thread-safe order book manager
#[derive(Debug)]
pub struct OrderBookManager {
    books: Arc<RwLock<std::collections::HashMap<String, OrderBook>>>,
    max_depth: usize,
}

impl OrderBookManager {
    /// Create a new order book manager
    pub fn new(max_depth: usize) -> Self {
        Self {
            books: Arc::new(RwLock::new(std::collections::HashMap::new())),
            max_depth,
        }
    }

    /// Get or create an order book for a token
    pub fn get_or_create_book(&self, token_id: &str) -> Result<OrderBook> {
        let mut books = self.books.write().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        if let Some(book) = books.get(token_id) {
            Ok(book.clone())
        } else {
            let book = OrderBook::new(token_id.to_string(), self.max_depth);
            books.insert(token_id.to_string(), book.clone());
            Ok(book)
        }
    }

    /// Update a book with a delta
    pub fn apply_delta(&self, delta: OrderDelta) -> Result<()> {
        let mut books = self.books.write().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        let book = books
            .get_mut(&delta.token_id)
            .ok_or_else(|| {
                PolyfillError::market_data(
                    format!("No book found for token: {}", delta.token_id),
                    crate::errors::MarketDataErrorKind::TokenNotFound,
                )
            })?;

        book.apply_delta(delta)
    }

    /// Get a book snapshot
    pub fn get_book(&self, token_id: &str) -> Result<crate::types::OrderBook> {
        let books = self.books.read().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        books
            .get(token_id)
            .map(|book| book.snapshot())
            .ok_or_else(|| {
                PolyfillError::market_data(
                    format!("No book found for token: {}", token_id),
                    crate::errors::MarketDataErrorKind::TokenNotFound,
                )
            })
    }

    /// Get all available books
    pub fn get_all_books(&self) -> Result<Vec<crate::types::OrderBook>> {
        let books = self.books.read().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        Ok(books.values().map(|book| book.snapshot()).collect())
    }

    /// Remove stale books
    pub fn cleanup_stale_books(&self, max_age: std::time::Duration) -> Result<usize> {
        let mut books = self.books.write().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        let initial_count = books.len();
        books.retain(|_, book| !book.is_stale(max_age));
        let removed = initial_count - books.len();

        if removed > 0 {
            debug!("Removed {} stale order books", removed);
        }

        Ok(removed)
    }
}

/// Order book analytics and statistics
#[derive(Debug, Clone)]
pub struct BookAnalytics {
    pub token_id: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub bid_count: usize,
    pub ask_count: usize,
    pub total_bid_size: Decimal,
    pub total_ask_size: Decimal,
    pub spread: Option<Decimal>,
    pub spread_pct: Option<Decimal>,
    pub mid_price: Option<Decimal>,
    pub volatility: Option<Decimal>,
}

impl OrderBook {
    /// Calculate analytics for this book
    pub fn analytics(&self) -> BookAnalytics {
        let bid_count = self.bids.len();
        let ask_count = self.asks.len();
        let total_bid_size: Decimal = self.bids.values().sum();
        let total_ask_size: Decimal = self.asks.values().sum();

        BookAnalytics {
            token_id: self.token_id.clone(),
            timestamp: self.timestamp,
            bid_count,
            ask_count,
            total_bid_size,
            total_ask_size,
            spread: self.spread(),
            spread_pct: self.spread_pct(),
            mid_price: self.mid_price(),
            volatility: self.calculate_volatility(),
        }
    }

    /// Calculate price volatility (simplified)
    fn calculate_volatility(&self) -> Option<Decimal> {
        // This is a simplified volatility calculation
        // In a real implementation, you'd want to track price history
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_order_book_creation() {
        let book = OrderBook::new("test_token".to_string(), 10);
        assert_eq!(book.token_id, "test_token");
        assert_eq!(book.bids.len(), 0);
        assert_eq!(book.asks.len(), 0);
    }

    #[test]
    fn test_apply_delta() {
        let mut book = OrderBook::new("test_token".to_string(), 10);
        
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: Utc::now(),
            side: Side::BUY,
            price: dec!(0.5),
            size: dec!(100),
            sequence: 1,
        };

        book.apply_delta(delta).unwrap();
        assert_eq!(book.sequence, 1);
        assert_eq!(book.best_bid().unwrap().price, dec!(0.5));
        assert_eq!(book.best_bid().unwrap().size, dec!(100));
    }

    #[test]
    fn test_spread_calculation() {
        let mut book = OrderBook::new("test_token".to_string(), 10);
        
        // Add bid
        book.apply_delta(OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: Utc::now(),
            side: Side::BUY,
            price: dec!(0.5),
            size: dec!(100),
            sequence: 1,
        }).unwrap();

        // Add ask
        book.apply_delta(OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: Utc::now(),
            side: Side::SELL,
            price: dec!(0.52),
            size: dec!(100),
            sequence: 2,
        }).unwrap();

        let spread = book.spread().unwrap();
        assert_eq!(spread, dec!(0.02));
    }

    #[test]
    fn test_market_impact() {
        let mut book = OrderBook::new("test_token".to_string(), 10);
        
        // Add multiple ask levels
        for (i, price) in [dec!(0.50), dec!(0.51), dec!(0.52)].iter().enumerate() {
            book.apply_delta(OrderDelta {
                token_id: "test_token".to_string(),
                timestamp: Utc::now(),
                side: Side::SELL,
                price: *price,
                size: dec!(100),
                sequence: i as u64 + 1,
            }).unwrap();
        }

        let impact = book.calculate_market_impact(Side::BUY, dec!(150)).unwrap();
        assert!(impact.average_price > dec!(0.50));
        assert!(impact.average_price < dec!(0.51));
    }
} 