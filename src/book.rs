//! Order book management for Polymarket client

use crate::errors::{PolyfillError, Result};
use crate::types::*;
use crate::utils::math;
use rust_decimal::Decimal;
use std::collections::BTreeMap; // BTreeMap keeps prices sorted automatically - crucial for order books
use std::sync::{Arc, RwLock}; // For thread-safe access across multiple tasks
use tracing::{debug, trace, warn}; // Logging for debugging and monitoring
use chrono::Utc;
use std::collections::HashMap;

/// High-performance order book implementation
/// 
/// This is the core data structure that holds all the live buy/sell orders for a token.
/// The efficiency of this code is critical as the order book is constantly being updated as orders are added and removed.
#[derive(Debug, Clone)]
pub struct OrderBook {
    /// Token ID this book represents (like "123456" for a specific prediction market outcome)
    pub token_id: String,
    
    /// Current sequence number for ordering updates
    /// This helps us ignore old/duplicate updates that arrive out of order
    pub sequence: u64,
    
    /// Last update timestamp - when we last got new data for this book
    pub timestamp: chrono::DateTime<Utc>,
    
    /// Bid side (price -> size, sorted descending)
    /// BTreeMap automatically keeps highest bids first, which is what we want
    /// Key = price (like 0.65), Value = total size at that price (like 1000 tokens)
    bids: BTreeMap<Decimal, Decimal>,
    
    /// Ask side (price -> size, sorted ascending) 
    /// BTreeMap keeps lowest asks first - people selling at cheapest prices
    asks: BTreeMap<Decimal, Decimal>,
    
    /// Minimum tick size for this market (like 0.01 = prices must be in penny increments)
    /// Some markets only allow certain price increments
    tick_size: Option<Decimal>,
    
    /// Maximum depth to maintain (how many price levels to keep)
    /// 
    /// We don't need to track every single price level, just the best ones because:
    /// - Trading reality 90% of volume happens in the top 5-10 price levels
    /// - Execution priority: Orders get filled from best price first, so deep levels often don't matter
    /// - Market efficiency: If you're buying and best ask is $0.67, you'll never pay $0.95
    /// - Risk management: Large orders that would hit deep levels are usually broken up
    /// - Data freshness: Deep levels often have stale orders from hours/days ago
    /// 
    /// Typical values: 10-50 for retail, 100-500 for institutional HFT systems
    max_depth: usize,
}

impl OrderBook {
    /// Create a new order book
    /// Just sets up empty bid/ask maps and basic metadata
    pub fn new(token_id: String, max_depth: usize) -> Self {
        Self {
            token_id,
            sequence: 0, // Start at 0, will increment as we get updates
            timestamp: Utc::now(),
            bids: BTreeMap::new(), // Empty to start
            asks: BTreeMap::new(), // Empty to start
            tick_size: None, // We'll set this later when we learn about the market
            max_depth,
        }
    }

    /// Set the tick size for this book
    /// This tells us the minimum price increment allowed (like 0.01 for penny increments)
    pub fn set_tick_size(&mut self, tick_size: Decimal) {
        self.tick_size = Some(tick_size);
    }

    /// Get the current best bid (highest price someone is willing to pay)
    /// Uses next_back() because BTreeMap sorts ascending, but we want the highest bid
    pub fn best_bid(&self) -> Option<BookLevel> {
        self.bids.iter().next_back().map(|(&price, &size)| BookLevel { price, size })
    }

    /// Get the current best ask (lowest price someone is willing to sell at)
    /// Uses next() because BTreeMap sorts ascending, so first item is lowest ask
    pub fn best_ask(&self) -> Option<BookLevel> {
        self.asks.iter().next().map(|(&price, &size)| BookLevel { price, size })
    }

    /// Get the current spread (difference between best ask and best bid)
    /// This tells us how "tight" the market is - smaller spread = more liquid market
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask.price - bid.price),
            _ => None, // Can't calculate spread if we're missing bid or ask
        }
    }

    /// Get the current mid price (halfway between best bid and ask)
    /// This is often used as the "fair value" of the market
    pub fn mid_price(&self) -> Option<Decimal> {
        math::mid_price(
            self.best_bid()?.price,
            self.best_ask()?.price,
        )
    }

    /// Get the spread as a percentage (relative to the bid price)
    /// Useful for comparing spreads across different price levels
    pub fn spread_pct(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => math::spread_pct(bid.price, ask.price),
            _ => None,
        }
    }

    /// Get all bids up to a certain depth (top N price levels)
    /// Returns them in descending price order (best bids first)
    pub fn bids(&self, depth: Option<usize>) -> Vec<BookLevel> {
        let depth = depth.unwrap_or(self.max_depth);
        self.bids
            .iter()
            .rev() // Reverse because we want highest prices first
            .take(depth) // Only take the top N levels
            .map(|(&price, &size)| BookLevel { price, size })
            .collect()
    }

    /// Get all asks up to a certain depth (top N price levels)
    /// Returns them in ascending price order (best asks first)
    pub fn asks(&self, depth: Option<usize>) -> Vec<BookLevel> {
        let depth = depth.unwrap_or(self.max_depth);
        self.asks
            .iter() // Already in ascending order, so no need to reverse
            .take(depth) // Only take the top N levels
            .map(|(&price, &size)| BookLevel { price, size })
            .collect()
    }

    /// Get the full book snapshot
    /// Creates a copy of the current state that can be safely passed around
    /// without worrying about the original book changing
    pub fn snapshot(&self) -> crate::types::OrderBook {
        crate::types::OrderBook {
            token_id: self.token_id.clone(),
            timestamp: self.timestamp,
            bids: self.bids(None), // Get all bids (up to max_depth)
            asks: self.asks(None), // Get all asks (up to max_depth)
            sequence: self.sequence,
        }
    }

    /// Apply a delta update to the book
    /// A "delta" is an incremental change - like "add 100 tokens at $0.65" or "remove all at $0.70"
    pub fn apply_delta(&mut self, delta: OrderDelta) -> Result<()> {
        // Validate sequence ordering - ignore old updates that arrive late
        // This is crucial for maintaining data integrity in real-time systems
        if delta.sequence <= self.sequence {
            trace!("Ignoring stale delta: {} <= {}", delta.sequence, self.sequence);
            return Ok(());
        }

        // Update our tracking info
        self.sequence = delta.sequence;
        self.timestamp = delta.timestamp;

        // Apply the actual change to the appropriate side
        match delta.side {
            Side::BUY => self.apply_bid_delta(delta.price, delta.size),
            Side::SELL => self.apply_ask_delta(delta.price, delta.size),
        }

        // Keep the book from getting too deep (memory management)
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

    /// Apply a bid-side delta (someone wants to buy)
    /// If size is 0, it means "remove this price level entirely"
    /// Otherwise, set the total size at this price level
    fn apply_bid_delta(&mut self, price: Decimal, size: Decimal) {
        if size.is_zero() {
            self.bids.remove(&price); // No more buyers at this price
        } else {
            self.bids.insert(price, size); // Update total size at this price
        }
    }

    /// Apply an ask-side delta (someone wants to sell)
    /// Same logic as bids - size of 0 means remove the price level
    fn apply_ask_delta(&mut self, price: Decimal, size: Decimal) {
        if size.is_zero() {
            self.asks.remove(&price); // No more sellers at this price
        } else {
            self.asks.insert(price, size); // Update total size at this price
        }
    }

    /// Trim the book to maintain depth limits
    /// We don't want to track every single price level - just the best ones
    /// 
    /// Why limit depth? Several reasons:
    /// 1. Memory efficiency: A popular market might have thousands of price levels,
    ///    but only the top 10-50 levels are actually tradeable with reasonable size
    /// 2. Performance: Fewer levels = faster iteration when calculating market impact
    /// 3. Relevance: Deep levels (like bids at $0.01 when best bid is $0.65) are 
    ///    mostly noise and will never get hit in normal trading
    /// 4. Stale data: Deep levels often contain old orders that haven't been cancelled
    /// 5. Network bandwidth: Less data to send when streaming updates

    fn trim_depth(&mut self) {
        // For bids, remove the LOWEST prices (worst bids) if we have too many
        // Example: If best bid is $0.65, we don't care about bids at $0.10
        if self.bids.len() > self.max_depth {
            let to_remove = self.bids.len() - self.max_depth;
            for _ in 0..to_remove {
                self.bids.pop_first(); // Remove lowest bid prices (furthest from market)
            }
        }

        // For asks, remove the HIGHEST prices (worst asks) if we have too many  
        // Example: If best ask is $0.67, we don't care about asks at $0.95
        if self.asks.len() > self.max_depth {
            let to_remove = self.asks.len() - self.max_depth;
            for _ in 0..to_remove {
                self.asks.pop_last(); // Remove highest ask prices (furthest from market)
            }
        }
    }

    /// Calculate the market impact for a given order size
    /// This is exactly why we don't need deep levels - if your order would require
    /// hitting prices way off the current market (like $0.95 when best ask is $0.67),
    /// you'd never actually place that order. You'd either:
    /// 1. Break it into smaller pieces over time
    /// 2. Use a different trading strategy
    /// 3. Accept that there's not enough liquidity right now
    pub fn calculate_market_impact(&self, side: Side, size: Decimal) -> Option<MarketImpact> {
        // Get the levels we'd be trading against
        let levels = match side {
            Side::BUY => self.asks(None),   // If buying, we hit the ask side
            Side::SELL => self.bids(None),  // If selling, we hit the bid side
        };

        if levels.is_empty() {
            return None; // No liquidity available
        }

        let mut remaining_size = size;
        let mut total_cost = Decimal::ZERO;
        let mut weighted_price = Decimal::ZERO;

        // Walk through each price level, filling as much as we can
        for level in levels {
            let fill_size = std::cmp::min(remaining_size, level.size);
            let level_cost = fill_size * level.price;
            
            total_cost += level_cost;
            weighted_price += level_cost; // This accumulates the weighted average
            remaining_size -= fill_size;

            if remaining_size.is_zero() {
                break; // We've filled our entire order
            }
        }

        if remaining_size > Decimal::ZERO {
            // Not enough liquidity to fill the whole order
            // This is a perfect example of why we don't need infinite depth:
            // If we can't fill your order with the top N levels, you probably
            // shouldn't be placing that order anyway - it would move the market too much
            return None; 
        }

        let avg_price = weighted_price / size;
        
        // Calculate how much we moved the market compared to the best price
        let impact = match side {
            Side::BUY => {
                let best_ask = self.best_ask()?.price;
                (avg_price - best_ask) / best_ask // How much worse than best ask
            }
            Side::SELL => {
                let best_bid = self.best_bid()?.price;
                (best_bid - avg_price) / best_bid // How much worse than best bid
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
    /// Useful for detecting when we've lost connection to live data
    pub fn is_stale(&self, max_age: std::time::Duration) -> bool {
        let age = Utc::now() - self.timestamp;
        age > chrono::Duration::from_std(max_age).unwrap_or_default()
    }

    /// Get the total liquidity at a given price level
    /// Tells you how much you can buy/sell at exactly this price
    pub fn liquidity_at_price(&self, price: Decimal, side: Side) -> Decimal {
        match side {
            Side::BUY => self.asks.get(&price).copied().unwrap_or_default(), // How much we can buy at this price
            Side::SELL => self.bids.get(&price).copied().unwrap_or_default(), // How much we can sell at this price
        }
    }

    /// Get the total liquidity within a price range
    /// Useful for understanding how much depth exists in a certain price band
    pub fn liquidity_in_range(&self, min_price: Decimal, max_price: Decimal, side: Side) -> Decimal {
        let levels: Vec<_> = match side {
            Side::BUY => self.asks.range(min_price..=max_price).collect(),
            Side::SELL => self.bids.range(min_price..=max_price).rev().collect(),
        };

        levels.into_iter().map(|(_, &size)| size).sum()
    }

    /// Validate that prices are properly ordered
    /// A healthy book should have best bid < best ask (otherwise there's an arbitrage opportunity)
    pub fn is_valid(&self) -> bool {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => bid.price < ask.price, // Normal market condition
            _ => true, // Empty book is technically valid
        }
    }
}

/// Market impact calculation result
/// This tells you what would happen if you executed a large order
#[derive(Debug, Clone)]
pub struct MarketImpact {
    pub average_price: Decimal,  // The average price you'd get across all fills
    pub impact_pct: Decimal,     // How much worse than the best price (as percentage)
    pub total_cost: Decimal,     // Total amount you'd pay/receive
    pub size_filled: Decimal,    // How much of your order got filled
}

/// Thread-safe order book manager
/// This manages multiple order books (one per token) and handles concurrent access
/// Multiple threads can read/write different books simultaneously
/// 
/// The depth limiting becomes even more critical here because we might be tracking
/// hundreds or thousands of different tokens simultaneously. If each book had
/// unlimited depth, we could easily use gigabytes of RAM for mostly useless data.
/// 
/// Example: 1000 tokens × 1000 price levels × 32 bytes per level = 32MB just for prices
/// With depth limiting: 1000 tokens × 50 levels × 32 bytes = 1.6MB (20x less memory)
#[derive(Debug)]
pub struct OrderBookManager {
    books: Arc<RwLock<std::collections::HashMap<String, OrderBook>>>, // Token ID -> OrderBook
    max_depth: usize,
}

impl OrderBookManager {
    /// Create a new order book manager
    /// Starts with an empty collection of books
    pub fn new(max_depth: usize) -> Self {
        Self {
            books: Arc::new(RwLock::new(std::collections::HashMap::new())),
            max_depth,
        }
    }

    /// Get or create an order book for a token
    /// If we don't have a book for this token yet, create a new empty one
    pub fn get_or_create_book(&self, token_id: &str) -> Result<OrderBook> {
        let mut books = self.books.write().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        if let Some(book) = books.get(token_id) {
            Ok(book.clone()) // Return a copy of the existing book
        } else {
            // Create a new book for this token
            let book = OrderBook::new(token_id.to_string(), self.max_depth);
            books.insert(token_id.to_string(), book.clone());
            Ok(book)
        }
    }

    /// Update a book with a delta
    /// This is called when we receive real-time updates from the exchange
    pub fn apply_delta(&self, delta: OrderDelta) -> Result<()> {
        let mut books = self.books.write().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        // Find the book for this token (must already exist)
        let book = books
            .get_mut(&delta.token_id)
            .ok_or_else(|| {
                PolyfillError::market_data(
                    format!("No book found for token: {}", delta.token_id),
                    crate::errors::MarketDataErrorKind::TokenNotFound,
                )
            })?;

        // Apply the update to the specific book
        book.apply_delta(delta)
    }

    /// Get a book snapshot
    /// Returns a copy of the current book state that won't change
    pub fn get_book(&self, token_id: &str) -> Result<crate::types::OrderBook> {
        let books = self.books.read().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        books
            .get(token_id)
            .map(|book| book.snapshot()) // Create a snapshot copy
            .ok_or_else(|| {
                PolyfillError::market_data(
                    format!("No book found for token: {}", token_id),
                    crate::errors::MarketDataErrorKind::TokenNotFound,
                )
            })
    }

    /// Get all available books
    /// Returns snapshots of every book we're currently tracking
    pub fn get_all_books(&self) -> Result<Vec<crate::types::OrderBook>> {
        let books = self.books.read().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        Ok(books.values().map(|book| book.snapshot()).collect())
    }

    /// Remove stale books
    /// Cleans up books that haven't been updated recently (probably disconnected)
    /// This prevents memory leaks from accumulating dead books
    pub fn cleanup_stale_books(&self, max_age: std::time::Duration) -> Result<usize> {
        let mut books = self.books.write().map_err(|_| {
            PolyfillError::internal_simple("Failed to acquire book lock")
        })?;

        let initial_count = books.len();
        books.retain(|_, book| !book.is_stale(max_age)); // Keep only non-stale books
        let removed = initial_count - books.len();

        if removed > 0 {
            debug!("Removed {} stale order books", removed);
        }

        Ok(removed)
    }
}

/// Order book analytics and statistics
/// Provides a summary view of the book's health and characteristics
#[derive(Debug, Clone)]
pub struct BookAnalytics {
    pub token_id: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub bid_count: usize,          // How many different bid price levels
    pub ask_count: usize,          // How many different ask price levels
    pub total_bid_size: Decimal,   // Total size of all bids combined
    pub total_ask_size: Decimal,   // Total size of all asks combined
    pub spread: Option<Decimal>,   // Current spread (ask - bid)
    pub spread_pct: Option<Decimal>, // Spread as percentage
    pub mid_price: Option<Decimal>, // Current mid price
    pub volatility: Option<Decimal>, // Price volatility (if calculated)
}

impl OrderBook {
    /// Calculate analytics for this book
    /// Gives you a quick health check of the market
    pub fn analytics(&self) -> BookAnalytics {
        let bid_count = self.bids.len();
        let ask_count = self.asks.len();
        let total_bid_size: Decimal = self.bids.values().sum(); // Add up all bid sizes
        let total_ask_size: Decimal = self.asks.values().sum(); // Add up all ask sizes

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
    /// This is a placeholder - real volatility needs historical price data
    fn calculate_volatility(&self) -> Option<Decimal> {
        // This is a simplified volatility calculation
        // In a real implementation, you'd want to track price history over time
        // and calculate standard deviation of price changes
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec; // Convenient macro for creating Decimal literals

    #[test]
    fn test_order_book_creation() {
        // Test that we can create a new empty order book
        let book = OrderBook::new("test_token".to_string(), 10);
        assert_eq!(book.token_id, "test_token");
        assert_eq!(book.bids.len(), 0); // Should start empty
        assert_eq!(book.asks.len(), 0); // Should start empty
    }

    #[test]
    fn test_apply_delta() {
        // Test that we can apply order book updates
        let mut book = OrderBook::new("test_token".to_string(), 10);
        
        // Create a buy order at $0.50 for 100 tokens
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: Utc::now(),
            side: Side::BUY,
            price: dec!(0.5),
            size: dec!(100),
            sequence: 1,
        };

        book.apply_delta(delta).unwrap();
        assert_eq!(book.sequence, 1); // Sequence should update
        assert_eq!(book.best_bid().unwrap().price, dec!(0.5)); // Should be our bid
        assert_eq!(book.best_bid().unwrap().size, dec!(100)); // Should be our size
    }

    #[test]
    fn test_spread_calculation() {
        // Test that we can calculate the spread between bid and ask
        let mut book = OrderBook::new("test_token".to_string(), 10);
        
        // Add a bid at $0.50
        book.apply_delta(OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: Utc::now(),
            side: Side::BUY,
            price: dec!(0.5),
            size: dec!(100),
            sequence: 1,
        }).unwrap();

        // Add an ask at $0.52
        book.apply_delta(OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: Utc::now(),
            side: Side::SELL,
            price: dec!(0.52),
            size: dec!(100),
            sequence: 2,
        }).unwrap();

        let spread = book.spread().unwrap();
        assert_eq!(spread, dec!(0.02)); // $0.52 - $0.50 = $0.02
    }

    #[test]
    fn test_market_impact() {
        // Test market impact calculation for a large order
        let mut book = OrderBook::new("test_token".to_string(), 10);
        
        // Add multiple ask levels (people selling at different prices)
        // $0.50 for 100 tokens, $0.51 for 100 tokens, $0.52 for 100 tokens
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

        // Try to buy 150 tokens (will need to hit multiple price levels)
        let impact = book.calculate_market_impact(Side::BUY, dec!(150)).unwrap();
        assert!(impact.average_price > dec!(0.50)); // Should be worse than best price
        assert!(impact.average_price < dec!(0.51)); // But not as bad as second level
    }
} 