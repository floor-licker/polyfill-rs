//! Snipe example for polyfill-rs
//!
//! This example demonstrates high-frequency trading techniques including:
//! - Real-time order book monitoring
//! - Stale quote detection
//! - Rapid order execution
//! - Market impact analysis

use polyfill_rs::{
    book::OrderBookManager,
    errors::Result,
    fill::{FillEngine, FillStatus},
    types::*,
    utils::time,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use rust_decimal::prelude::ToPrimitive;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Snipe trading strategy
#[derive(Debug)]
pub struct SnipeStrategy {
    /// Target token ID
    token_id: String,
    /// Maximum spread to consider
    max_spread_pct: Decimal,
    /// Minimum order size
    min_order_size: Decimal,
    /// Maximum order size
    max_order_size: Decimal,
    /// Stale quote threshold (seconds)
    stale_threshold: u64,
    /// Last known best prices
    last_best_bid: Option<Decimal>,
    last_best_ask: Option<Decimal>,
    /// Last update timestamp
    last_update: u64,
    /// Order book manager
    book_manager: OrderBookManager,
    /// Fill engine
    fill_engine: FillEngine,
    /// Statistics
    stats: SnipeStats,
}

/// Snipe trading statistics
#[derive(Debug, Clone)]
pub struct SnipeStats {
    pub opportunities_detected: u64,
    pub orders_placed: u64,
    pub orders_filled: u64,
    pub total_volume: Decimal,
    pub total_pnl: Decimal,
    pub avg_fill_time_ms: f64,
}

impl Default for SnipeStats {
    fn default() -> Self {
        Self {
            opportunities_detected: 0,
            orders_placed: 0,
            orders_filled: 0,
            total_volume: dec!(0),
            total_pnl: dec!(0),
            avg_fill_time_ms: 0.0,
        }
    }
}

impl SnipeStrategy {
    /// Create a new snipe strategy
    pub fn new(
        token_id: String,
        max_spread_pct: Decimal,
        min_order_size: Decimal,
        max_order_size: Decimal,
        stale_threshold: u64,
    ) -> Self {
        Self {
            token_id,
            max_spread_pct,
            min_order_size,
            max_order_size,
            stale_threshold,
            last_best_bid: None,
            last_best_ask: None,
            last_update: 0,
            book_manager: OrderBookManager::new(100),
            fill_engine: FillEngine::new(
                min_order_size,
                dec!(2.0), // 2% max slippage
                5,          // 5 bps fee rate
            ),
            stats: SnipeStats::default(),
        }
    }

    /// Process a market data update
    pub fn process_update(&mut self, message: StreamMessage) -> Result<()> {
        match message {
            StreamMessage::BookUpdate { data } => {
                if data.token_id == self.token_id {
                    self.process_book_update(data)?;
                }
            }
            StreamMessage::Trade { data } => {
                if data.token_id == self.token_id {
                    self.process_trade(data)?;
                }
            }
            StreamMessage::Heartbeat { timestamp: _ } => {
                self.check_stale_quotes()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Process order book update
    fn process_book_update(&mut self, delta: OrderDelta) -> Result<()> {
        // Ensure book exists
        self.book_manager.get_or_create_book(&self.token_id)?;
        
        // Update local order book
        self.book_manager.apply_delta(delta.clone())?;

        // Get current book state
        let book = self.book_manager.get_book(&self.token_id)?;
        
        // Update best prices
        if let Some(best_bid) = book.bids.first() {
            self.last_best_bid = Some(best_bid.price);
        }
        if let Some(best_ask) = book.asks.first() {
            self.last_best_ask = Some(best_ask.price);
        }
        
        self.last_update = time::now_secs();

        // Check for trading opportunities
        self.check_opportunities()?;

        Ok(())
    }

    /// Process trade update
    fn process_trade(&mut self, fill: FillEvent) -> Result<()> {
        info!(
            "Trade: {} {} @ {} (size: {})",
            fill.side.as_str(),
            fill.token_id,
            fill.price,
            fill.size
        );

        // Update statistics
        self.stats.total_volume += fill.size;
        
        // Calculate P&L if this was our trade
        // (In a real implementation, you'd track your own orders)
        
        Ok(())
    }

    /// Check for trading opportunities
    fn check_opportunities(&mut self) -> Result<()> {
        let (bid, ask) = match (self.last_best_bid, self.last_best_ask) {
            (Some(bid), Some(ask)) => (bid, ask),
            _ => return Ok(()), // No liquidity
        };

        // Calculate spread
        let spread_pct = match (bid, ask) {
            (bid, ask) if bid > dec!(0) && ask > bid => {
                (ask - bid) / bid * dec!(100)
            }
            _ => return Ok(()),
        };

        // Check if spread is within our target
        if spread_pct <= self.max_spread_pct {
            self.stats.opportunities_detected += 1;
            
            info!(
                "Opportunity detected: spread {}% (target: {}%)",
                spread_pct, self.max_spread_pct
            );

            // Execute snipe order
            self.execute_snipe_order(bid, ask)?;
        }

        Ok(())
    }

    /// Execute a snipe order
    fn execute_snipe_order(&mut self, bid: Decimal, ask: Decimal) -> Result<()> {
        // Calculate order size (random between min and max)
        let random_factor = Decimal::from(rand::random::<u64>() % 100) / Decimal::from(100);
        let size = self.min_order_size + 
            (self.max_order_size - self.min_order_size) * random_factor;

        // Determine side based on market conditions
        let side = if bid > ask {
            Side::Sell // Crossed market, sell
        } else {
            Side::Buy // Normal market, buy
        };

        // Create market order request
        let request = MarketOrderRequest {
            token_id: self.token_id.clone(),
            side,
            amount: size,
            slippage_tolerance: Some(dec!(1.0)), // 1% slippage tolerance
            client_id: Some(format!("snipe_{}", time::now_millis())),
        };

        // Get current book for execution simulation
        let book = self.book_manager.get_book(&self.token_id)?;
        let mut book_impl = polyfill_rs::book::OrderBook::new(self.token_id.clone(), 100);
        
        // Convert to internal book format
        for level in &book.bids {
            book_impl.apply_delta(OrderDelta {
                token_id: self.token_id.clone(),
                timestamp: chrono::Utc::now(),
                side: Side::Buy,
                price: level.price,
                size: level.size,
                sequence: 1,
            })?;
        }
        
        for level in &book.asks {
            book_impl.apply_delta(OrderDelta {
                token_id: self.token_id.clone(),
                timestamp: chrono::Utc::now(),
                side: Side::Sell,
                price: level.price,
                size: level.size,
                sequence: 2,
            })?;
        }

        // Execute order
        let start_time = std::time::Instant::now();
        let result = self.fill_engine.execute_market_order(&request, &book_impl)?;
        let fill_time = start_time.elapsed().as_millis() as f64;

        // Update statistics
        self.stats.orders_placed += 1;
        if result.status == FillStatus::Filled {
            self.stats.orders_filled += 1;
        }

        // Update average fill time
        let total_time = self.stats.avg_fill_time_ms * (self.stats.orders_filled - 1) as f64 + fill_time;
        self.stats.avg_fill_time_ms = total_time / self.stats.orders_filled as f64;

        info!(
            "Snipe order executed: {} {} @ {} (fill time: {}ms)",
            result.total_size,
            side.as_str(),
            result.average_price,
            fill_time
        );

        Ok(())
    }

    /// Check for stale quotes
    fn check_stale_quotes(&mut self) -> Result<()> {
        let now = time::now_secs();
        let age = now.saturating_sub(self.last_update);

        if age > self.stale_threshold {
            warn!(
                "Stale quotes detected: {}s old (threshold: {}s)",
                age, self.stale_threshold
            );

            // In a real implementation, you might:
            // - Cancel pending orders
            // - Switch to a different data source
            // - Reduce position sizes
            // - Stop trading temporarily
        }

        Ok(())
    }

    /// Get current statistics
    pub fn get_stats(&self) -> &SnipeStats {
        &self.stats
    }
}

/// Mock market data generator for testing
struct MockMarketData {
    token_id: String,
    base_price: Decimal,
    volatility: Decimal,
    sequence: u64,
}

impl MockMarketData {
    fn new(token_id: String, base_price: Decimal) -> Self {
        Self {
            token_id,
            base_price,
            volatility: dec!(0.01), // 1% volatility
            sequence: 0,
        }
    }

    fn generate_update(&mut self) -> StreamMessage {
        self.sequence += 1;

        // Generate random price movement
        let random_factor = Decimal::from(rand::random::<i64>() % 100 - 50) / Decimal::from(100);
        let volatility_f64 = self.volatility.to_f64().unwrap_or(0.01);
        let price_change = random_factor * Decimal::from(2) * self.volatility;
        let new_price = self.base_price * (Decimal::from(1) + price_change);

        // Generate order book update
        let side = if rand::random::<bool>() { Side::Buy } else { Side::Sell };
        let size = Decimal::from(rand::random::<u64>() % 1000 + 100);

        StreamMessage::BookUpdate {
            data: OrderDelta {
                token_id: self.token_id.clone(),
                timestamp: chrono::Utc::now(),
                side,
                price: new_price,
                size,
                sequence: self.sequence,
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting snipe trading example...");

    // Create snipe strategy
    let mut strategy = SnipeStrategy::new(
        "12345".to_string(), // Example token ID
        dec!(2.0),           // 2% max spread
        dec!(10),            // Min order size
        dec!(100),           // Max order size
        5,                   // 5 second stale threshold
    );

    // Create mock market data generator
    let mut market_data = MockMarketData::new(
        "12345".to_string(),
        dec!(0.5), // Base price $0.50
    );

    // Simulate market data stream
    let mut message_count = 0;
    let max_messages = 100;

    while message_count < max_messages {
        // Generate market update
        let update = market_data.generate_update();
        
        // Process update
        if let Err(e) = strategy.process_update(update) {
            error!("Error processing update: {}", e);
        }

        // Print statistics every 10 messages
        if message_count % 10 == 0 {
            let stats = strategy.get_stats();
            info!(
                "Stats: {} opportunities, {} orders placed, {} filled, avg fill time: {:.2}ms",
                stats.opportunities_detected,
                stats.orders_placed,
                stats.orders_filled,
                stats.avg_fill_time_ms
            );
        }

        message_count += 1;
        sleep(Duration::from_millis(100)).await; // 100ms between updates
    }

    // Print final statistics
    let final_stats = strategy.get_stats();
    info!("Final statistics:");
    info!("  Opportunities detected: {}", final_stats.opportunities_detected);
    info!("  Orders placed: {}", final_stats.orders_placed);
    info!("  Orders filled: {}", final_stats.orders_filled);
    info!("  Total volume: {}", final_stats.total_volume);
    info!("  Average fill time: {:.2}ms", final_stats.avg_fill_time_ms);

    info!("Snipe trading example completed!");
    Ok(())
} 