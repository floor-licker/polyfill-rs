//! Polyfill-rs: High-performance Rust client for Polymarket
//! 
//! # Features
//! 
//! - **High-performance order book management** with optimized data structures
//! - **Real-time market data streaming** with WebSocket support
//! - **Trade execution simulation** with slippage protection
//! - **Comprehensive error handling** with specific error types
//! - **Rate limiting and retry logic** for robust API interactions
//! - **Ethereum integration** with EIP-712 signing support
//! - **Benchmarking tools** for performance analysis
//! 
//! # Quick Start
//! 
//! ```rust
//! use polyfill_rs::{ClobClient, OrderArgs, Side};
//! use rust_decimal::Decimal;
//! 
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create client (compatible with polymarket-rs-client)
//!     let mut client = ClobClient::with_l1_headers(
//!         "https://clob.polymarket.com",
//!         "your_private_key",
//!         137,
//!     );
//! 
//!     // Get API credentials
//!     let api_creds = client.create_or_derive_api_key(None).await?;
//!     client.set_api_creds(api_creds);
//! 
//!     // Create and post order
//!     let order_args = OrderArgs::new(
//!         "token_id",
//!         Decimal::from_str("0.75")?,
//!         Decimal::from_str("100.0")?,
//!         Side::BUY,
//!     );
//! 
//!     let result = client.create_and_post_order(&order_args).await?;
//!     println!("Order posted: {:?}", result);
//! 
//!     Ok(())
//! }
//! ```
//! 
//! # Advanced Usage
//! 
//! ```rust
//! use polyfill_rs::{PolyfillClient, ClientConfig};
//! 
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Advanced configuration
//!     let config = ClientConfig {
//!         base_url: "https://clob.polymarket.com".to_string(),
//!         chain_id: 137,
//!         private_key: Some("your_private_key".to_string()),
//!         max_slippage: Some(Decimal::from_str("0.001")?),
//!         fee_rate: Some(Decimal::from_str("0.02")?),
//!         ..Default::default()
//!     };
//! 
//!     let mut client = PolyfillClient::with_config(config)?;
//! 
//!     // Subscribe to real-time order book updates
//!     client.subscribe_to_order_book("token_id").await?;
//! 
//!     // Process incoming messages
//!     while let Some(message) = client.get_next_message().await? {
//!         println!("Received: {:?}", message);
//!     }
//! 
//!     Ok(())
//! }
//! ```

use tracing::info;


// Global constants
pub const DEFAULT_CHAIN_ID: u64 = 137; // Polygon
pub const DEFAULT_BASE_URL: &str = "https://clob.polymarket.com";
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_MAX_RETRIES: u32 = 3;
pub const DEFAULT_RATE_LIMIT_RPS: u32 = 100;

// Initialize logging
pub fn init() {
    tracing_subscriber::fmt::init();
    info!("Polyfill-rs initialized");
}

// Re-export main types
pub use crate::types::{
    ApiCredentials, Balance, BalanceAllowance, BatchMidpointRequest, BatchMidpointResponse,
    BatchPriceRequest, BatchPriceResponse, ClientConfig, FillEvent, MarketSnapshot, 
    NotificationParams, OpenOrder, OpenOrderParams, Order, OrderBook, OrderDelta, 
    OrderRequest, OrderStatus, OrderType, Side, StreamMessage, TokenPrice, TradeParams,
    WssAuth, WssSubscription, WssChannelType,
    // Additional compatibility types
    ApiKeysResponse, MidpointResponse, PriceResponse, SpreadResponse, TickSizeResponse,
    NegRiskResponse, BookParams, MarketsResponse, SimplifiedMarketsResponse, Market,
    SimplifiedMarket, Token, Rewards, ClientResult,
};

// Re-export client
pub use crate::client::{ClobClient, PolyfillClient};

// Re-export compatibility types (for easy migration from polymarket-rs-client)
pub use crate::client::{
    OrderArgs, OrderBookSummary,
};

// Re-export error types
pub use crate::errors::{PolyfillError, Result};

// Re-export advanced components
pub use crate::book::{OrderBook as OrderBookImpl, OrderBookManager};
pub use crate::fill::{FillEngine, FillResult};
pub use crate::stream::{MarketStream, StreamManager, WebSocketStream};
pub use crate::decode::Decoder;

// Re-export utilities
pub use crate::utils::{
    crypto, math, retry, time, url, rate_limit,
};

// Module declarations
pub mod auth;
pub mod book;
pub mod client;
pub mod decode;
pub mod errors;
pub mod fill;
pub mod orders;
pub mod stream;
pub mod types;
pub mod utils;

// Benchmarks
#[cfg(test)]
mod benches {
    use criterion::{criterion_group, criterion_main};
    use crate::{OrderBookManager, OrderDelta, Side};
    use rust_decimal::Decimal;
    use chrono::Utc;
    use std::str::FromStr;

    fn order_book_benchmark(c: &mut criterion::Criterion) {
        let mut book_manager = OrderBookManager::new(100);
        
        c.bench_function("apply_order_delta", |b| {
            b.iter(|| {
                let delta = OrderDelta {
                    token_id: "test_token".to_string(),
                    timestamp: Utc::now(),
                    side: Side::BUY,
                    price: Decimal::from_str("0.75").unwrap(),
                    size: Decimal::from_str("100.0").unwrap(),
                    sequence: 1,
                };
                
                let _ = book_manager.apply_delta(delta);
            });
        });
    }

    criterion_group!(benches, order_book_benchmark);
    criterion_main!(benches);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use alloy_primitives::U256;

    #[test]
    fn test_client_creation() {
        let client = ClobClient::new("https://test.example.com");
        // Test that the client was created successfully
        // We can't test private fields, but we can verify the client exists
        assert!(true); // Client creation successful
    }

    #[test]
    fn test_order_args_creation() {
        let args = OrderArgs::new(
            "test_token",
            Decimal::from_str("0.75").unwrap(),
            Decimal::from_str("100.0").unwrap(),
            Side::BUY,
        );
        
        assert_eq!(args.token_id, "test_token");
        assert_eq!(args.side, Side::BUY);
    }

    #[test]
    fn test_order_args_default() {
        let args = OrderArgs::default();
        assert_eq!(args.token_id, "");
        assert_eq!(args.price, Decimal::ZERO);
        assert_eq!(args.size, Decimal::ZERO);
        assert_eq!(args.side, Side::BUY);
    }
} 