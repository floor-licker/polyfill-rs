//! Coinbase Exchange WebSocket integration
//!
//! This module provides high-performance level2 orderbook streaming from
//! Coinbase Exchange. It uses the same optimizations as the rest of polyfill-rs:
//!
//! - SIMD-accelerated JSON parsing (via simd_json)
//! - Fixed-point integer arithmetic for orderbook operations
//! - Automatic reconnection with exponential backoff
//! - Local orderbook state management
//!
//! # Channel Options
//!
//! - `subscribe_batch()` - Uses `level2_batch` channel (50ms batches, **no auth required**)
//! - `subscribe()` - Uses `level2` channel (real-time, **requires authentication**)
//!
//! For most use cases, `subscribe_batch()` is recommended as it provides low-latency
//! orderbook updates without requiring API credentials.
//!
//! # Example
//!
//! ```rust,no_run
//! use polyfill_rs::coinbase::CoinbaseStream;
//! use futures::StreamExt;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut stream = CoinbaseStream::new(vec!["BTC-USD".to_string()]);
//!     stream.connect().await?;
//!     stream.subscribe_batch().await?;  // No auth required
//!
//!     while let Some(msg) = stream.next().await {
//!         match msg? {
//!             polyfill_rs::coinbase::Message::Snapshot(_) => {
//!                 println!("Received initial orderbook snapshot");
//!             }
//!             polyfill_rs::coinbase::Message::L2Update(_) => {
//!                 if let Some(book) = stream.book("BTC-USD") {
//!                     if let Some(bid) = book.best_bid() {
//!                         println!("Best bid: {} @ {}", bid.size, bid.price);
//!                     }
//!                 }
//!             }
//!             _ => {}
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod decode;
pub mod stream;
pub mod types;

// Re-export main types for convenience
pub use stream::{CoinbaseStream, DEFAULT_URL};
pub use types::{
    ErrorMessage, FastDelta, FastL2Update, FastSnapshot, Heartbeat, L2Update, Match, Message,
    Snapshot, Subscribe, Subscriptions, Unsubscribe,
};
