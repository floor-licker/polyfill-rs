# Polyfill-rs

A high-performance, low-latency Rust client for Polymarket optimized for high-frequency trading.

## Overview

Polyfill-rs provides a comprehensive trading infrastructure for algorithmic trading strategies on Polymarket's prediction markets. The library is designed for institutional-grade trading systems requiring high throughput and robust error handling.

**Key Features:**
- **Drop-in replacement** for `polymarket-rs-client` with enhanced functionality
- **High-performance order book management** with O(log n) operations
- **Real-time market data streaming** with WebSocket support
- **Trade execution simulation** with slippage protection
- **Comprehensive error handling** with specific error types

## Architecture

### Core Components

**Order Book Management**
- Real-time order book maintenance with `O(log n)` operations
- Thread-safe concurrent access patterns
- Market impact calculation and liquidity analysis
- Snapshot generation for strategy backtesting

**Trade Execution Engine**
- Market order simulation with slippage protection
- Limit order placement and management
- Fill event processing and tracking
- Fee calculation and cost analysis

**Streaming Infrastructure**
- WebSocket-based real-time market data feeds
- Automatic reconnection with exponential backoff
- Message parsing and validation
- Multi-stream management for concurrent market monitoring

**Client Interface**
- REST API integration for order management
- Authentication and signature generation
- Rate limiting and request throttling
- Comprehensive error handling with retry logic

## Performance Characteristics

### Latency Optimization
- Zero-copy data structures where possible
- Lock-free concurrent access patterns
- Minimal allocation in hot paths
- SIMD-optimized mathematical operations

### Throughput Capabilities
- High-frequency order book updates (10,000+ updates/second)
- Concurrent stream processing
- Efficient memory management with object pooling
- Optimized serialization/deserialization

### Memory Efficiency
- Compact data representations
- Minimal heap allocations
- Efficient string handling
- Memory-mapped data structures for large datasets

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
polyfill-rs = "0.1.0"
```

## Usage

### Basic Client Initialization (Compatible with polymarket-rs-client)

```rust
use polyfill_rs::{ClobClient, OrderArgs, Side};
use rust_decimal::Decimal;

let mut client = ClobClient::with_l1_headers(
    "https://clob.polymarket.com",
    "your_private_key",
    137,
);

// Get API credentials
let api_creds = client.create_or_derive_api_key(None).await?;
client.set_api_creds(api_creds);

// Create and post order
let order_args = OrderArgs::new(
    "token_id",
    Decimal::from_str("0.75")?,
    Decimal::from_str("100.0")?,
    Side::BUY,
);

let result = client.create_and_post_order(&order_args).await?;
```

### Advanced Features (Polyfill-rs specific)

```rust
use polyfill_rs::{PolyfillClient, ClientConfig};

// Advanced configuration
let config = ClientConfig {
    base_url: "https://clob.polymarket.com".to_string(),
    chain_id: 137,
    private_key: Some("your_private_key".to_string()),
    max_slippage: Some(Decimal::from_str("0.001")?),
    fee_rate: Some(Decimal::from_str("0.02")?),
    ..Default::default()
};

let mut client = PolyfillClient::with_config(config)?;

// Subscribe to real-time order book updates
client.subscribe_to_order_book("token_id").await?;

// Process incoming messages
while let Some(message) = client.get_next_message().await? {
    println!("Received: {:?}", message);
}
```

### Order Book Management

```rust
use polyfill_rs::{OrderBookManager, OrderDelta, Side};

let mut book_manager = OrderBookManager::new();

// Apply order book delta
let delta = OrderDelta {
    token_id: "market_token".to_string(),
    timestamp: chrono::Utc::now(),
    side: Side::Buy,
    price: Decimal::from_str("0.75")?,
    size: Decimal::from_str("100.0")?,
    sequence: 1,
};

book_manager.apply_delta(delta)?;

// Retrieve order book state
let book = book_manager.get_book("market_token")?;
let best_bid = book.best_bid();
let best_ask = book.best_ask();
let spread = book.spread();
```

### Trade Execution Simulation

```rust
use polyfill_rs::{FillEngine, MarketOrderRequest};

let mut fill_engine = FillEngine::new(
    Decimal::from_str("0.001")?, // max_slippage
    Decimal::from_str("0.02")?,  // fee_rate
);

let order = MarketOrderRequest {
    token_id: "market_token".to_string(),
    side: Side::Buy,
    size: Decimal::from_str("50.0")?,
    max_price: Some(Decimal::from_str("0.80")?),
};

let result = fill_engine.execute_market_order(&book, order)?;
println!("Filled: {} at avg price: {}", result.filled_size, result.average_price);
```

### Real-time Market Data

```rust
use polyfill_rs::{StreamManager, WebSocketStream};

let mut stream_manager = StreamManager::new();

// Subscribe to order book updates
let stream = WebSocketStream::new("wss://clob.polymarket.com/ws").await?;
stream_manager.add_stream("orderbook", stream).await?;

// Process incoming messages
while let Some(message) = stream_manager.next().await {
    match message {
        Ok(msg) => {
            // Process order book update
            if let Some(delta) = msg.to_order_delta() {
                book_manager.apply_delta(delta)?;
            }
        }
        Err(e) => {
            // Handle connection errors
            eprintln!("Stream error: {}", e);
        }
    }
}
```

### Demo Trading Strategy

```rust
use polyfill_rs::{PolyfillClient, OrderBookManager, FillEngine};

struct ArbitrageStrategy {
    client: PolyfillClient,
    book_manager: OrderBookManager,
    fill_engine: FillEngine,
    min_spread: Decimal,
    position_size: Decimal,
}

impl ArbitrageStrategy {
    async fn execute_arbitrage(&mut self, token_id: &str) -> Result<()> {
        let book = self.book_manager.get_book(token_id)?;
        
        // Calculate arbitrage opportunity
        let spread = book.spread();
        if spread < self.min_spread {
            return Ok(());
        }
        
        let mid_price = book.mid_price();
        let bid_price = book.best_bid().unwrap().price;
        let ask_price = book.best_ask().unwrap().price;
        
        // Execute cross-spread orders
        let buy_order = self.client.create_order(
            token_id,
            Side::Buy,
            self.position_size,
            Some(bid_price),
        ).await?;
        
        let sell_order = self.client.create_order(
            token_id,
            Side::Sell,
            self.position_size,
            Some(ask_price),
        ).await?;
        
        Ok(())
    }
}
```

## Configuration

### Performance Tuning

```rust
use polyfill_rs::Config;

let config = Config {
    // Network configuration
    base_url: "https://clob.polymarket.com".to_string(),
    chain_id: 137,
    
    // Authentication
    private_key: Some("your_private_key".to_string()),
    api_credentials: None,
    
    // Performance settings
    connection_timeout: Duration::from_secs(5),
    request_timeout: Duration::from_secs(10),
    max_retries: 3,
    retry_delay: Duration::from_millis(100),
    
    // Rate limiting
    requests_per_second: 100,
    burst_size: 10,
};
```

### Order Book Configuration

```rust
use polyfill_rs::OrderBookManager;

let book_manager = OrderBookManager::with_config(OrderBookConfig {
    max_books: 1000,
    cleanup_interval: Duration::from_secs(300),
    max_sequence_gap: 1000,
});
```

## Error Handling

The library provides comprehensive error handling with specific error types:

```rust
use polyfill_rs::errors::{PolyfillError, ErrorKind};

match result {
    Ok(data) => {
        // Process successful response
    }
    Err(PolyfillError::Network { .. }) => {
        // Handle network connectivity issues
    }
    Err(PolyfillError::RateLimit { retry_after, .. }) => {
        // Implement exponential backoff
        tokio::time::sleep(retry_after).await;
    }
    Err(PolyfillError::Order { order_id, .. }) => {
        // Handle order-specific errors
    }
    Err(e) => {
        // Handle other errors
        eprintln!("Unexpected error: {}", e);
    }
}
```