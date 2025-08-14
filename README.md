# Polyfill-rs

A blazing-fast Rust client for Polymarket that's actually built for people who need to process thousands of market updates per second.

## Overview

If you've ever tried to build a trading bot for prediction markets, you know the pain: existing libraries are either too slow, too basic, or both. Polyfill-rs fixes that.

This started as a drop-in replacement for `polymarket-rs-client`, but then I went down a rabbit hole optimizing everything. 

**What makes it different:**
- **Actually fast**: We replaced the slow parts with fixed-point math (benchmarks coming soon)
- **Built for real trading**: Designed to handle thousands of market updates per second
- **Easy to use**: Same API as the original library, just way faster under the hood
- **Teaches you**: Every decision is documented with code and explanations of why it matters

## How It Works

The library has four main pieces that work together:

### Order Book Engine
This is where the magic happens. Instead of using slow decimal math like everyone else, we use fixed-point integers internally:

- **Before**: `BTreeMap<Decimal, Decimal>` (slow decimal operations + allocations)
- **After**: `BTreeMap<u32, i64>` (fast integer operations, zero allocations)

The order book can process updates much faster because integer comparisons are fundamentally faster than decimal ones. We only convert back to decimals when you actually need the data.

*Want to see how this works?* Check out `src/book.rs` - every optimization has commented-out "before" code so you can see exactly what changed and why.

### Trade Execution Simulator
Want to know what would happen if you bought 1000 tokens right now? This simulates walking through the order book levels:

```rust
let impact = book.calculate_market_impact(Side::BUY, Decimal::from(1000));
// Tells you: average price, total cost, market impact percentage
```

It's smart about slippage protection and won't let you accidentally market-buy at ridiculous prices.

### Real-Time Data Streaming
WebSocket connections that don't give up. When the connection drops (and it will), the library automatically reconnects with exponential backoff. No more babysitting your data feeds.

### HTTP Client
All the boring stuff like authentication, rate limiting, and retry logic. It just works so you don't have to think about it.

## Performance (Benchmarks Coming Soon)

The library is designed around several key optimizations:

### Order Book Operations
- **Fixed-point math**: Integer operations instead of decimal arithmetic
- **Zero allocations**: Reuse data structures in hot paths
- **Efficient lookups**: Optimized data structures for common operations
- **Batch processing**: Handle multiple updates efficiently

### Memory Efficiency
- **Compact representations**: Smaller memory footprint per price level
- **Controlled depth**: Only track relevant price levels
- **Smart cleanup**: Remove stale data automatically

### Design Philosophy
The core insight is that most trading operations don't need full decimal precision during intermediate calculations. By using fixed-point integers internally and only converting to decimals at the API boundaries, we can:

- Eliminate allocation overhead in hot paths
- Use faster integer arithmetic
- Reduce memory usage significantly
- Maintain full precision where it matters

**Learning from the code**: The performance optimizations are documented with detailed comments explaining the math, memory layout, and algorithmic choices. It's like a mini-course in high-frequency trading optimization.

## Getting Started

```toml
[dependencies]
polyfill-rs = "0.1.0"
```

## Basic Usage

### If You're Coming From polymarket-rs-client

Good news: your existing code should work without changes. I kept the same API.

```rust
use polyfill_rs::{ClobClient, OrderArgs, Side};
use rust_decimal::Decimal;

// Same initialization as before
let mut client = ClobClient::with_l1_headers(
    "https://clob.polymarket.com",
    "your_private_key",
    137,
);

// Same API calls
let api_creds = client.create_or_derive_api_key(None).await?;
client.set_api_creds(api_creds);

// Same order creation
let order_args = OrderArgs::new(
    "token_id",
    Decimal::from_str("0.75")?,
    Decimal::from_str("100.0")?,
    Side::BUY,
);

let result = client.create_and_post_order(&order_args).await?;
```

The difference is that this now runs way faster under the hood.

### Real-Time Order Book Tracking

Here's where it gets interesting. You can track live order books for multiple tokens:

```rust
use polyfill_rs::{OrderBookManager, OrderDelta, Side};

let mut book_manager = OrderBookManager::new(50); // Keep top 50 price levels

// This is what happens when you get a WebSocket update
let delta = OrderDelta {
    token_id: "market_token".to_string(),
    timestamp: chrono::Utc::now(),
    side: Side::BUY,
    price: Decimal::from_str("0.75")?,
    size: Decimal::from_str("100.0")?,  // 0 means remove this price level
    sequence: 1,
};

book_manager.apply_delta(delta)?;  // This is now super fast

// Get current market state
let book = book_manager.get_book("market_token")?;
let spread = book.spread();           // How tight is the market?
let mid_price = book.mid_price();     // Fair value estimate
let best_bid = book.best_bid();       // Highest buy price
let best_ask = book.best_ask();       // Lowest sell price
```

The `apply_delta` call used to be the bottleneck. Now it's basically free.

### Market Impact Analysis

Before you place a big order, you probably want to know what it'll cost you:

```rust
use polyfill_rs::FillEngine;

let mut fill_engine = FillEngine::new(
    Decimal::from_str("0.001")?, // max slippage: 0.1%
    Decimal::from_str("0.02")?,  // fee rate: 2%
    10,                          // fee in basis points
);

// Simulate buying $1000 worth
let order = MarketOrderRequest {
    token_id: "market_token".to_string(),
    side: Side::BUY,
    amount: Decimal::from_str("1000.0")?,
    slippage_tolerance: Some(Decimal::from_str("0.005")?), // 0.5%
    client_id: None,
};

let result = fill_engine.execute_market_order(&order, &book)?;

println!("If you bought $1000 worth right now:");
println!("- Average price: ${}", result.average_price);
println!("- Total tokens: {}", result.total_size);
println!("- Fees: ${}", result.fees);
println!("- Market impact: {}%", result.impact_pct * 100);
```

This tells you exactly what would happen without actually placing the order. Super useful for position sizing.

### WebSocket Streaming (The Fun Part)

Here's how you connect to live market data. The library handles all the annoying reconnection stuff:

```rust
use polyfill_rs::{WebSocketStream, StreamManager};

let mut stream = WebSocketStream::new("wss://clob.polymarket.com/ws");

// Set up authentication (you'll need API credentials)
let auth = WssAuth {
    address: "your_eth_address".to_string(),
    signature: "your_signature".to_string(),
    timestamp: chrono::Utc::now().timestamp() as u64,
    nonce: "random_nonce".to_string(),
};
stream = stream.with_auth(auth);

// Subscribe to specific markets
stream.subscribe_market_channel(vec!["token_id_1".to_string(), "token_id_2".to_string()]).await?;

// Process live updates
while let Some(message) = stream.next().await {
    match message? {
        StreamMessage::MarketBookUpdate { data } => {
            // This is where the fast order book updates happen
            book_manager.apply_delta_fast(data)?;
        }
        StreamMessage::MarketTrade { data } => {
            println!("Trade: {} tokens at ${}", data.size, data.price);
        }
        StreamMessage::Heartbeat { .. } => {
            // Connection is alive
        }
        _ => {}
    }
}
```

The stream automatically reconnects when it drops. You just keep processing messages.

### Example: Simple Spread Trading Bot

Here's a basic bot that looks for wide spreads and tries to capture them:

```rust
use polyfill_rs::{ClobClient, OrderBookManager, FillEngine};

struct SpreadBot {
    client: ClobClient,
    book_manager: OrderBookManager,
    min_spread_pct: Decimal,  // Only trade if spread > this %
    position_size: Decimal,   // How much to trade each time
}

impl SpreadBot {
    async fn check_opportunity(&mut self, token_id: &str) -> Result<bool> {
        let book = self.book_manager.get_book(token_id)?;
        
        // Get current market state
        let spread_pct = book.spread_pct().unwrap_or_default();
        let best_bid = book.best_bid();
        let best_ask = book.best_ask();
        
        // Only trade if spread is wide enough and we have liquidity
        if spread_pct > self.min_spread_pct && best_bid.is_some() && best_ask.is_some() {
            println!("Found opportunity: {}% spread on {}", spread_pct, token_id);
            
            // Check if our order size would move the market too much
            let impact = book.calculate_market_impact(Side::BUY, self.position_size);
            if let Some(impact) = impact {
                if impact.impact_pct < Decimal::from_str("0.01")? { // < 1% impact
                    return Ok(true);
                }
            }
        }
        
        Ok(false)
    }
    
    async fn execute_trade(&mut self, token_id: &str) -> Result<()> {
        // This is where you'd actually place orders
        // Left as an exercise for the reader :)
        println!("Would place orders for {}", token_id);
        Ok(())
    }
}
```

The key insight: with fast order book updates, you can check hundreds of tokens for opportunities without the library being the bottleneck.

**Pro tip**: The trading strategy examples in the code include detailed comments about market microstructure, order flow, and risk management techniques.

## Configuration Tips

### Order Book Depth Settings

The most important performance knob is how many price levels to track:

```rust
// For most trading bots: 10-50 levels is plenty
let book_manager = OrderBookManager::new(20);

// For market making: maybe 100+ levels
let book_manager = OrderBookManager::new(100);

// For analysis/research: could go higher, but memory usage grows
let book_manager = OrderBookManager::new(500);
```

Why this matters: Each price level takes memory, but 90% of trading happens in the top 10 levels anyway. More levels = more memory usage for diminishing returns.

*The code comments in `src/book.rs` explain the memory layout and why we chose these specific data structures for different use cases.*

### WebSocket Reconnection

The defaults are pretty good, but you can tune them:

```rust
let reconnect_config = ReconnectConfig {
    max_retries: 5,                                    // Give up after 5 attempts
    base_delay: Duration::from_secs(1),               // Start with 1 second delay
    max_delay: Duration::from_secs(60),               // Cap at 1 minute
    backoff_multiplier: 2.0,                          // Double delay each time
};

let stream = WebSocketStream::new("wss://clob.polymarket.com/ws")
    .with_reconnect_config(reconnect_config);
```

### Memory Usage

If you're tracking lots of tokens, you might want to clean up stale books:

```rust
// Remove books that haven't updated in 5 minutes
let removed = book_manager.cleanup_stale_books(Duration::from_secs(300))?;
println!("Cleaned up {} stale order books", removed);
```

## Error Handling (Because Things Break)

The library tries to be helpful about what went wrong:

```rust
use polyfill_rs::errors::PolyfillError;

match book_manager.apply_delta(delta) {
    Ok(_) => {
        // Order book updated successfully
    }
    Err(PolyfillError::Validation { message, .. }) => {
        // Bad data (price not aligned to tick size, etc.)
        eprintln!("Invalid data: {}", message);
    }
    Err(PolyfillError::Network { .. }) => {
        // Network problems - probably worth retrying
        eprintln!("Network error, will retry...");
    }
    Err(PolyfillError::RateLimit { retry_after, .. }) => {
        // Hit rate limits - back off
        if let Some(delay) = retry_after {
            tokio::time::sleep(delay).await;
        }
    }
    Err(PolyfillError::Stream { kind, .. }) => {
        // WebSocket issues - the library will try to reconnect automatically
        eprintln!("Stream error: {:?}", kind);
    }
    Err(e) => {
        eprintln!("Something else went wrong: {}", e);
    }
}
```

Most errors tell you whether they're worth retrying or if you should give up.

## What's Different From Other Libraries?

### Performance
Most trading libraries are built for "demo day" - they work fine for small examples but fall apart under real load. This one is designed for people who actually need to process thousands of updates per second.

### Tick Alignment
The library enforces price tick alignment automatically. If someone sends you a price that doesn't align to the market's tick size (like $0.6543 when the tick size is $0.01), it gets rejected. This prevents weird pricing bugs.

*The tick alignment code includes detailed comments about why this matters for market integrity and how the integer math makes validation nearly free.*

### Memory Management
Order books can grow huge if you're not careful. The library automatically trims them to keep only the relevant price levels, and you can clean up stale books that haven't updated recently.

## Contributing

Found a bug? Have a performance improvement? PRs welcome!

The codebase is designed to be educational as well as functional. Every optimization includes:
- Commented-out "before" code showing the slower approach
- Detailed explanations of why the optimization works
- Performance measurements and memory usage analysis
- References to trading concepts and market microstructure theory

If you're curious about high-frequency trading or high performance Rust, start with `src/book.rs` - it's like a textbook on order book performance engineering.