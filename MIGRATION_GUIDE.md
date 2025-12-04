# Migration Guide: From polymarket-rs-client to polyfill-rs

This guide helps you migrate from the original `polymarket-rs-client` to our high-performance `polyfill-rs` implementation.

## Quick Migration (Drop-in Replacement)

### 1. Update Cargo.toml

**Before:**
```toml
[dependencies]
polymarket-rs-client = "0.x.x"
```

**After:**
```toml
[dependencies]
polyfill-rs = "0.1.0"
```

### 2. Update Imports

**Before:**
```rust
use polymarket_rs_client::{ClobClient, Side, OrderType, OrderArgs};
```

**After:**
```rust
use polyfill_rs::{ClobClient, Side, OrderType, OrderArgs};
```

### 3. Code Remains Identical

All your existing code continues to work without changes:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Same API, same functionality
    let client = ClobClient::new("https://clob.polymarket.com");
    let markets = client.get_sampling_markets(None).await?;
    
    // All methods work identically
    let order_args = OrderArgs::new("token_id", price, size, Side::BUY);
    let order = client.create_order(&order_args, None, None, None).await?;
    let result = client.post_order(order, OrderType::GTC).await?;
    
    Ok(())
}
```

## What You Get with polyfill-rs

### ✅ 100% API Compatibility
- All 49 methods from the original client
- Identical method signatures and return types
- Same authentication and error handling patterns

### 🚀 Performance Improvements
- **Fixed-point arithmetic** for order book operations (up to 10x faster)
- **Zero-allocation** hot paths for high-frequency trading
- **Memory-efficient** order book management
- **Optimized** data structures for trading operations

### 🔥 Additional Features
- **WebSocket streaming** for real-time market data
- **Advanced fill processing** and execution tracking
- **Comprehensive metrics** collection
- **Robust reconnection** handling for WebSocket connections

## Advanced Usage (Optional Enhancements)

If you want to leverage the additional features:

### WebSocket Streaming
```rust
use polyfill_rs::{WebSocketStream, StreamMessage};

let mut stream = WebSocketStream::new("wss://ws-subscriptions-clob.polymarket.com").await?;
stream.subscribe_to_market("market_id").await?;

while let Some(message) = stream.next().await {
    match message? {
        StreamMessage::OrderBookUpdate(update) => {
            // Handle real-time order book updates
        }
        StreamMessage::Trade(trade) => {
            // Handle trade events
        }
        _ => {}
    }
}
```

### High-Performance Order Book
```rust
use polyfill_rs::OrderBookImpl;

// Create order book with configurable depth for memory efficiency
let mut book = OrderBookImpl::new("token_id".to_string(), 100); // 100 levels max

// Fast fixed-point operations
let spread = book.spread_fast(); // Returns Option<u32> (ticks)
let mid = book.mid_fast();       // Returns Option<u32> (ticks)
```

### Fill Processing
```rust
use polyfill_rs::{FillEngine, FillProcessor};

let fill_engine = FillEngine::new();
let processor = FillProcessor::new(fill_engine);

// Track order executions with detailed metrics
processor.process_fill(fill_event).await?;
```

## Migration Checklist

- [ ] Update `Cargo.toml` dependency
- [ ] Update import statements
- [ ] Run tests to verify functionality
- [ ] (Optional) Leverage new WebSocket streaming features
- [ ] (Optional) Use high-performance order book operations
- [ ] (Optional) Implement fill processing for execution tracking

## Troubleshooting

### Compilation Issues
If you encounter compilation errors:

1. **Check Rust version**: Ensure you're using Rust 1.70+ (same as original client)
2. **Clear cache**: Run `cargo clean` and rebuild
3. **Update dependencies**: Run `cargo update`

### Runtime Differences
The only runtime differences are performance improvements:

- **Faster order book operations** (transparent to your code)
- **Lower memory usage** for order book management
- **Better error messages** with more context

### Getting Help
- Check our [API documentation](https://docs.rs/polyfill-rs)
- Review the [API Parity Report](./API_PARITY_REPORT.md)
- Open an issue on [GitHub](https://github.com/juliustranquilli/polyfill-rs)

## Why Migrate?

1. **Performance**: Significant speed improvements for trading operations
2. **Features**: Additional capabilities not available in the original
3. **Maintenance**: Actively maintained with regular updates
4. **Compatibility**: 100% drop-in replacement with zero code changes required
5. **Future-proof**: Built for high-frequency trading environments

The migration is risk-free since the API is identical, but you gain substantial performance benefits and additional features for advanced use cases.
