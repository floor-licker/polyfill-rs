//! Comprehensive Demo for polyfill-rs
//!
//! This example demonstrates all the major functions and capabilities of the polyfill-rs library:
//! - Basic client operations and API calls
//! - Order book management and analytics
//! - Real-time streaming capabilities
//! - Trade execution and fill processing
//! - Utility functions and mathematical operations
//! - Error handling and retry logic
//! - Rate limiting and performance optimizations

use polyfill_rs::{
    // Order book management
    book::{OrderBook, OrderBookManager},

    // Error handling
    errors::{PolyfillError, Result},

    // Fill execution
    fill::{FillEngine, FillProcessor},

    // Streaming capabilities
    stream::{StreamManager, WebSocketStream},

    // Types and structures
    types::*,

    // Utility functions
    utils::{address, crypto, math, rate_limit, retry, time, url},

    // Configuration
    ClientConfig,
    // Core client types
    ClobClient,
    OrderArgs,
    OrderType,

    PolyfillClient,
    Side,
};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info};

/// Comprehensive demo showcasing all polyfill-rs functionality
#[allow(dead_code)]
pub struct PolyfillDemo {
    /// Basic HTTP client
    client: ClobClient,
    /// Advanced client with configuration
    advanced_client: PolyfillClient,
    /// Order book manager
    book_manager: OrderBookManager,
    /// Fill engine for trade execution
    fill_engine: FillEngine,
    /// Fill processor for handling fills
    fill_processor: FillProcessor,
    /// Stream manager for real-time data
    stream_manager: StreamManager,
    /// Rate limiter
    rate_limiter: rate_limit::TokenBucket,
    /// Statistics
    stats: DemoStats,
}

/// Demo statistics
#[derive(Debug, Clone)]
pub struct DemoStats {
    pub api_calls: u64,
    pub orders_processed: u64,
    pub fills_processed: u64,
    pub stream_messages: u64,
    pub errors: u64,
    pub total_volume: Decimal,
}

impl Default for DemoStats {
    fn default() -> Self {
        Self {
            api_calls: 0,
            orders_processed: 0,
            fills_processed: 0,
            stream_messages: 0,
            errors: 0,
            total_volume: dec!(0),
        }
    }
}

impl PolyfillDemo {
    /// Create a new comprehensive demo
    pub fn new() -> Result<Self> {
        // Create basic client
        let client = ClobClient::new("https://clob.polymarket.com");

        // Create advanced client with configuration
        let _config = ClientConfig {
            base_url: "https://clob.polymarket.com".to_string(),
            chain_id: 137,                  // Polygon
            private_key: None,              // Would be set in production
            api_credentials: None,          // Would be set in production
            max_slippage: Some(dec!(0.01)), // 1% max slippage
            fee_rate: Some(dec!(0.02)),     // 2% fee rate
            timeout: Some(Duration::from_secs(30)),
            max_connections: Some(100),
        };

        let advanced_client = PolyfillClient::new("https://clob.polymarket.com");

        // Create order book manager
        let book_manager = OrderBookManager::new(100);

        // Create fill engine
        let fill_engine = FillEngine::new(
            dec!(1.0), // Min fill size
            dec!(2.0), // Max slippage 2%
            5,         // 5 bps fee rate
        );

        // Create fill processor
        let fill_processor = FillProcessor::new(1000);

        // Create stream manager
        let stream_manager = StreamManager::new();

        // Create rate limiter (100 requests per second)
        let rate_limiter = rate_limit::TokenBucket::new(100, 100);

        Ok(Self {
            client,
            advanced_client,
            book_manager,
            fill_engine,
            fill_processor,
            stream_manager,
            rate_limiter,
            stats: DemoStats::default(),
        })
    }

    /// Demo 1: Basic API Operations
    pub async fn demo_basic_api_operations(&mut self) -> Result<()> {
        info!("=== Demo 1: Basic API Operations ===");

        // Test connectivity
        let is_ok = self.client.get_ok().await;
        info!("API connectivity: {}", is_ok);
        self.stats.api_calls += 1;

        // Get server time
        match self.client.get_server_time().await {
            Ok(timestamp) => {
                info!("Server time: {}", timestamp);
                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get server time: {}", e);
                self.stats.errors += 1;
            },
        }

        // Get sampling markets
        match self.client.get_sampling_markets(None).await {
            Ok(markets) => {
                info!("Found {} markets", markets.data.len());
                for market in &markets.data[..std::cmp::min(3, markets.data.len())] {
                    info!("  Market: {} - {}", market.question, market.market_slug);
                }
                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get markets: {}", e);
                self.stats.errors += 1;
            },
        }

        Ok(())
    }

    /// Demo 2: Order Book Operations
    pub async fn demo_order_book_operations(&mut self) -> Result<()> {
        info!("=== Demo 2: Order Book Operations ===");

        // Example token ID (you would use a real one in production)
        let token_id = "12345";

        // Get order book from API
        match self.client.get_order_book(token_id).await {
            Ok(order_book) => {
                info!(
                    "Order book for token {}: {} bids, {} asks",
                    token_id,
                    order_book.bids.len(),
                    order_book.asks.len()
                );

                // Create local order book
                let mut local_book = OrderBook::new(token_id.to_string(), 50);

                // Apply order book data to local book
                for (i, bid) in order_book.bids.iter().enumerate() {
                    local_book.apply_delta(OrderDelta {
                        token_id: token_id.to_string(),
                        timestamp: chrono::Utc::now(),
                        side: Side::BUY,
                        price: bid.price,
                        size: bid.size,
                        sequence: i as u64,
                    })?;
                }

                for (i, ask) in order_book.asks.iter().enumerate() {
                    local_book.apply_delta(OrderDelta {
                        token_id: token_id.to_string(),
                        timestamp: chrono::Utc::now(),
                        side: Side::SELL,
                        price: ask.price,
                        size: ask.size,
                        sequence: (order_book.bids.len() + i) as u64,
                    })?;
                }

                // Get analytics
                let analytics = local_book.analytics();
                info!("Book analytics:");
                info!(
                    "  Bid levels: {}, Ask levels: {}",
                    analytics.bid_count, analytics.ask_count
                );
                info!(
                    "  Total bid size: {}, Total ask size: {}",
                    analytics.total_bid_size, analytics.total_ask_size
                );
                if let Some(spread) = analytics.spread {
                    info!(
                        "  Spread: {} ({:.2}%)",
                        spread,
                        analytics.spread_pct.unwrap_or(dec!(0))
                    );
                }
                if let Some(mid) = analytics.mid_price {
                    info!("  Mid price: {}", mid);
                }

                // Calculate market impact
                if let Some(impact) = local_book.calculate_market_impact(Side::BUY, dec!(100.0)) {
                    info!("Market impact for 100 size buy:");
                    info!("  Average price: {}", impact.average_price);
                    info!("  Impact: {:.2}%", impact.impact_pct);
                    info!("  Total cost: {}", impact.total_cost);
                }

                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get order book: {}", e);
                self.stats.errors += 1;
            },
        }

        Ok(())
    }

    /// Demo 3: Market Data Operations
    pub async fn demo_market_data_operations(&mut self) -> Result<()> {
        info!("=== Demo 3: Market Data Operations ===");

        let token_id = "12345";

        // Get midpoint
        match self.client.get_midpoint(token_id).await {
            Ok(midpoint) => {
                info!("Midpoint for {}: {}", token_id, midpoint.mid);
                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get midpoint: {}", e);
                self.stats.errors += 1;
            },
        }

        // Get spread
        match self.client.get_spread(token_id).await {
            Ok(spread) => {
                info!("Spread for {}: {}", token_id, spread.spread);
                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get spread: {}", e);
                self.stats.errors += 1;
            },
        }

        // Get price for both sides
        for side in [Side::BUY, Side::SELL] {
            match self.client.get_price(token_id, side).await {
                Ok(price) => {
                    info!("{} price for {}: {}", side.as_str(), token_id, price.price);
                    self.stats.api_calls += 1;
                },
                Err(e) => {
                    error!("Failed to get {} price: {}", side.as_str(), e);
                    self.stats.errors += 1;
                },
            }
        }

        // Get tick size
        match self.client.get_tick_size(token_id).await {
            Ok(tick_size) => {
                info!("Tick size for {}: {}", token_id, tick_size);
                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get tick size: {}", e);
                self.stats.errors += 1;
            },
        }

        // Get neg risk
        match self.client.get_neg_risk(token_id).await {
            Ok(neg_risk) => {
                info!("Neg risk for {}: {}", token_id, neg_risk);
                self.stats.api_calls += 1;
            },
            Err(e) => {
                error!("Failed to get neg risk: {}", e);
                self.stats.errors += 1;
            },
        }

        Ok(())
    }

    /// Demo 4: Order Creation and Management
    pub async fn demo_order_operations(&mut self) -> Result<()> {
        info!("=== Demo 4: Order Creation and Management ===");

        // Create order arguments
        let order_args = OrderArgs::new("12345", dec!(0.75), dec!(100.0), Side::BUY);

        info!("Created order args: {:?}", order_args);

        // Create market order request
        let market_order = MarketOrderRequest {
            token_id: "12345".to_string(),
            side: Side::BUY,
            amount: dec!(100.0),
            slippage_tolerance: Some(dec!(1.0)), // 1% slippage
            client_id: Some("demo_market_order".to_string()),
        };

        info!("Created market order request: {:?}", market_order);

        // Create limit order request
        let limit_order = OrderRequest {
            token_id: "12345".to_string(),
            side: Side::BUY,
            price: dec!(0.75),
            size: dec!(100.0),
            order_type: OrderType::GTC,
            expiration: None,
            client_id: Some("demo_limit_order".to_string()),
        };

        info!("Created limit order request: {:?}", limit_order);

        self.stats.orders_processed += 2;

        Ok(())
    }

    /// Demo 5: Fill Execution
    pub async fn demo_fill_execution(&mut self) -> Result<()> {
        info!("=== Demo 5: Fill Execution ===");

        // Create a mock order book for testing
        let mut book = OrderBook::new("12345".to_string(), 50);

        // Add some liquidity
        for i in 1..=5 {
            book.apply_delta(OrderDelta {
                token_id: "12345".to_string(),
                timestamp: chrono::Utc::now(),
                side: Side::BUY,
                price: dec!(0.70) + Decimal::from(i) * dec!(0.01),
                size: dec!(100.0),
                sequence: i,
            })?;
        }

        for i in 1..=5 {
            book.apply_delta(OrderDelta {
                token_id: "12345".to_string(),
                timestamp: chrono::Utc::now(),
                side: Side::SELL,
                price: dec!(0.80) + Decimal::from(i) * dec!(0.01),
                size: dec!(100.0),
                sequence: i + 10,
            })?;
        }

        info!("Created order book with liquidity");

        // Execute market order
        let market_order = MarketOrderRequest {
            token_id: "12345".to_string(),
            side: Side::BUY,
            amount: dec!(50.0),
            slippage_tolerance: Some(dec!(2.0)),
            client_id: Some("demo_market_buy".to_string()),
        };

        let fill_result = self
            .fill_engine
            .execute_market_order(&market_order, &book)?;

        info!("Market order execution result:");
        info!("  Status: {:?}", fill_result.status);
        info!("  Total size: {}", fill_result.total_size);
        info!("  Average price: {}", fill_result.average_price);
        info!("  Total cost: {}", fill_result.total_cost);
        info!("  Fees: {}", fill_result.fees);
        info!("  Number of fills: {}", fill_result.fills.len());

        // Process fills
        for fill in &fill_result.fills {
            self.fill_processor.process_fill(fill.clone())?;
            self.stats.fills_processed += 1;
            self.stats.total_volume += fill.size;
        }

        // Execute limit order
        let limit_order = OrderRequest {
            token_id: "12345".to_string(),
            side: Side::SELL,
            price: dec!(0.85),
            size: dec!(25.0),
            order_type: OrderType::GTC,
            expiration: None,
            client_id: Some("demo_limit_sell".to_string()),
        };

        let limit_result = self.fill_engine.execute_limit_order(&limit_order, &book)?;

        info!("Limit order execution result:");
        info!("  Status: {:?}", limit_result.status);
        info!("  Total size: {}", limit_result.total_size);
        info!("  Average price: {}", limit_result.average_price);

        self.stats.orders_processed += 2;

        Ok(())
    }

    /// Demo 6: Utility Functions
    pub async fn demo_utility_functions(&mut self) -> Result<()> {
        info!("=== Demo 6: Utility Functions ===");

        // Time utilities
        info!("Time utilities:");
        info!("  Current timestamp (secs): {}", time::now_secs());
        info!("  Current timestamp (millis): {}", time::now_millis());
        info!("  Current timestamp (micros): {}", time::now_micros());

        // Math utilities
        info!("Math utilities:");
        let price = dec!(0.7534);
        let tick_size = dec!(0.01);
        let rounded_price = math::round_to_tick(price, tick_size);
        info!(
            "  Price: {}, Tick size: {}, Rounded: {}",
            price, tick_size, rounded_price
        );

        let notional = math::notional(price, dec!(100.0));
        info!("  Notional value: {}", notional);

        let spread_pct = math::spread_pct(dec!(0.75), dec!(0.76));
        info!("  Spread percentage: {:?}", spread_pct);

        let mid_price = math::mid_price(dec!(0.75), dec!(0.76));
        info!("  Mid price: {:?}", mid_price);

        // Address utilities
        info!("Address utilities:");
        let address = "0x1234567890123456789012345678901234567890";
        match address::parse_address(address) {
            Ok(addr) => info!("  Parsed address: {:?}", addr),
            Err(e) => error!("  Failed to parse address: {}", e),
        }

        let token_id = "12345";
        match address::validate_token_id(token_id) {
            Ok(_) => info!("  Valid token ID: {}", token_id),
            Err(e) => error!("  Invalid token ID: {}", e),
        }

        // URL utilities
        info!("URL utilities:");
        let endpoint = url::build_endpoint("https://api.example.com", "/v1/orders")?;
        info!("  Built endpoint: {}", endpoint);

        // Rate limiting
        info!("Rate limiting:");
        for i in 0..5 {
            let allowed = self.rate_limiter.try_consume();
            info!(
                "  Request {}: {}",
                i + 1,
                if allowed { "ALLOWED" } else { "RATE LIMITED" }
            );
        }

        Ok(())
    }

    /// Demo 7: Error Handling and Retry Logic
    pub async fn demo_error_handling(&mut self) -> Result<()> {
        info!("=== Demo 7: Error Handling and Retry Logic ===");

        // Demonstrate retry logic
        let retry_config = retry::RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
            backoff_factor: 2.0,
            jitter: true,
        };

        let operation = || async {
            // Simulate a potentially failing operation
            if rand::random::<bool>() {
                Ok("Success!")
            } else {
                Err(PolyfillError::network(
                    "Simulated network error",
                    std::io::Error::other("Simulated error"),
                ))
            }
        };

        match retry::with_retry(&retry_config, operation).await {
            Ok(result) => {
                info!("Retry operation succeeded: {}", result);
            },
            Err(e) => {
                error!("Retry operation failed after all attempts: {}", e);
                self.stats.errors += 1;
            },
        }

        // Demonstrate error types
        info!("Error types demonstration:");

        let api_error = PolyfillError::api(400, "Bad Request");
        info!("  API Error: {:?}", api_error);

        let network_error = PolyfillError::network(
            "Connection timeout",
            std::io::Error::new(std::io::ErrorKind::TimedOut, "Connection timeout"),
        );
        info!("  Network Error: {:?}", network_error);

        let parse_error = PolyfillError::parse("Invalid JSON", None);
        info!("  Parse Error: {:?}", parse_error);

        let config_error = PolyfillError::config("Invalid configuration");
        info!("  Config Error: {:?}", config_error);

        Ok(())
    }

    /// Demo 8: Streaming Capabilities (Mock)
    pub async fn demo_streaming_capabilities(&mut self) -> Result<()> {
        info!("=== Demo 8: Streaming Capabilities ===");

        // Create a mock WebSocket stream
        let _stream = WebSocketStream::new("wss://ws-subscriptions-clob.polymarket.com/ws/market");

        info!("Created WebSocket stream");

        // Simulate subscription
        let subscription = WssSubscription {
            auth: WssAuth {
                address: "0x1234567890123456789012345678901234567890".to_string(),
                signature: "mock_signature".to_string(),
                timestamp: time::now_secs(),
                nonce: crypto::generate_nonce().to_string(),
            },
            markets: Some(vec!["market1".to_string(), "market2".to_string()]),
            asset_ids: Some(vec!["12345".to_string(), "67890".to_string()]),
            channel_type: "USER".to_string(),
        };

        info!("Created subscription: {:?}", subscription);

        // Simulate receiving stream messages
        let messages = vec![
            StreamMessage::Heartbeat {
                timestamp: chrono::Utc::now(),
            },
            StreamMessage::BookUpdate {
                data: OrderDelta {
                    token_id: "12345".to_string(),
                    timestamp: chrono::Utc::now(),
                    side: Side::BUY,
                    price: dec!(0.75),
                    size: dec!(100.0),
                    sequence: 1,
                },
            },
            StreamMessage::Trade {
                data: FillEvent {
                    id: "fill1".to_string(),
                    order_id: "order1".to_string(),
                    token_id: "12345".to_string(),
                    side: Side::BUY,
                    price: dec!(0.75),
                    size: dec!(50.0),
                    timestamp: chrono::Utc::now(),
                    maker_address: alloy_primitives::Address::ZERO,
                    taker_address: alloy_primitives::Address::ZERO,
                    fee: dec!(0.375),
                },
            },
        ];

        for message in messages {
            info!("Received stream message: {:?}", message);
            self.stats.stream_messages += 1;

            // Process message based on type
            match &message {
                StreamMessage::BookUpdate { data } => {
                    info!("  Processing book update for token: {}", data.token_id);
                    if let Err(e) = self.book_manager.apply_delta(data.clone()) {
                        error!("  Failed to apply book update: {}", e);
                        self.stats.errors += 1;
                    }
                },
                StreamMessage::Trade { data } => {
                    info!(
                        "  Processing trade: {} {} @ {}",
                        data.side.as_str(),
                        data.size,
                        data.price
                    );
                    if let Err(e) = self.fill_processor.process_fill(data.clone()) {
                        error!("  Failed to process fill: {}", e);
                        self.stats.errors += 1;
                    }
                },
                StreamMessage::Heartbeat { timestamp } => {
                    debug!("  Received heartbeat at: {}", timestamp);
                },
                _ => {
                    info!("  Unhandled message type");
                },
            }
        }

        Ok(())
    }

    /// Demo 9: Performance and Analytics
    pub async fn demo_performance_analytics(&mut self) -> Result<()> {
        info!("=== Demo 9: Performance and Analytics ===");

        // Get fill engine statistics
        let fill_stats = self.fill_engine.get_stats();
        info!("Fill engine statistics:");
        info!("  Total orders: {}", fill_stats.total_orders);
        info!("  Total fills: {}", fill_stats.total_fills);
        info!("  Total volume: {}", fill_stats.total_volume);
        info!("  Total fees: {}", fill_stats.total_fees);

        // Get fill processor statistics
        let processor_stats = self.fill_processor.get_stats();
        info!("Fill processor statistics:");
        info!("  Pending orders: {}", processor_stats.pending_orders);
        info!("  Pending fills: {}", processor_stats.pending_fills);
        info!("  Pending volume: {}", processor_stats.pending_volume);
        info!("  Processed fills: {}", processor_stats.processed_fills);
        info!("  Processed volume: {}", processor_stats.processed_volume);

        // Get demo statistics
        info!("Demo statistics:");
        info!("  API calls: {}", self.stats.api_calls);
        info!("  Orders processed: {}", self.stats.orders_processed);
        info!("  Fills processed: {}", self.stats.fills_processed);
        info!("  Stream messages: {}", self.stats.stream_messages);
        info!("  Errors: {}", self.stats.errors);
        info!("  Total volume: {}", self.stats.total_volume);

        // Calculate error rate
        let total_operations =
            self.stats.api_calls + self.stats.orders_processed + self.stats.stream_messages;
        let error_rate = if total_operations > 0 {
            (self.stats.errors as f64 / total_operations as f64) * 100.0
        } else {
            0.0
        };
        info!("  Error rate: {:.2}%", error_rate);

        Ok(())
    }

    /// Run all demos
    pub async fn run_all_demos(&mut self) -> Result<()> {
        info!("Starting comprehensive polyfill-rs demo...");

        // Run all demo sections
        self.demo_basic_api_operations().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_order_book_operations().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_market_data_operations().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_order_operations().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_fill_execution().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_utility_functions().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_error_handling().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_streaming_capabilities().await?;
        sleep(Duration::from_millis(500)).await;

        self.demo_performance_analytics().await?;

        info!("Comprehensive demo completed successfully!");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Polyfill-rs Comprehensive Demo");
    info!("==============================");

    // Create and run demo
    let mut demo = PolyfillDemo::new()?;

    if let Err(e) = demo.run_all_demos().await {
        error!("Demo failed: {}", e);
        std::process::exit(1);
    }

    info!("Demo completed successfully!");
    Ok(())
}
