//! Test script for Coinbase BTC-USD orderbook streaming
//!
//! Run with: cargo run --example coinbase_btc

use futures::StreamExt;
use polyfill_rs::coinbase::{CoinbaseStream, Message};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("Connecting to Coinbase WebSocket...");

    let mut stream = CoinbaseStream::new(vec!["BTC-USD".to_string()]).with_max_depth(50); // Keep top 50 levels

    stream.connect().await?;
    // Use subscribe_batch() for unauthenticated access (50ms batches)
    // Use subscribe() for real-time updates (requires auth)
    stream.subscribe_batch().await?;

    println!("Subscribed to BTC-USD level2_batch channel (50ms batches)");
    println!("Waiting for orderbook data...\n");

    let start = Instant::now();
    let mut update_count = 0u64;
    let mut last_print = Instant::now();

    while let Some(result) = stream.next().await {
        match result {
            Ok(msg) => {
                match msg {
                    Message::Snapshot(snapshot) => {
                        println!(
                            "Received snapshot: {} bids, {} asks",
                            snapshot.bids.len(),
                            snapshot.asks.len()
                        );
                    },
                    Message::L2Update(_) => {
                        update_count += 1;

                        // Print orderbook state every second
                        if last_print.elapsed().as_secs() >= 1 {
                            if let Some(book) = stream.book("BTC-USD") {
                                let best_bid = book.best_bid();
                                let best_ask = book.best_ask();

                                if let (Some(bid), Some(ask)) = (best_bid, best_ask) {
                                    let spread = &ask.price - &bid.price;
                                    let spread_bps =
                                        (&spread / &bid.price) * rust_decimal::Decimal::from(10000);

                                    println!(
                                        "BTC-USD | Bid: ${:.2} ({:.4}) | Ask: ${:.2} ({:.4}) | Spread: ${:.2} ({:.1} bps) | Updates: {} | Rate: {:.0}/s",
                                        bid.price,
                                        bid.size,
                                        ask.price,
                                        ask.size,
                                        spread,
                                        spread_bps,
                                        update_count,
                                        update_count as f64 / start.elapsed().as_secs_f64()
                                    );
                                }
                            }
                            last_print = Instant::now();
                        }
                    },
                    Message::Heartbeat(_) => {
                        // Heartbeats keep the connection alive
                    },
                    Message::Subscriptions(subs) => {
                        println!("Confirmed subscriptions: {:?}", subs.channels);
                    },
                    Message::Error(err) => {
                        eprintln!("Error from Coinbase: {} ({:?})", err.message, err.reason);
                    },
                }
            },
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            },
        }

        // Run for 10 seconds then exit
        if start.elapsed().as_secs() >= 10 {
            println!("\n10 seconds elapsed, disconnecting...");
            break;
        }
    }

    let stats = stream.get_stats();
    println!("\nFinal Statistics:");
    println!("  Messages received: {}", stats.messages_received);
    println!("  Messages sent: {}", stats.messages_sent);
    println!("  Errors: {}", stats.errors);
    println!("  Reconnects: {}", stats.reconnect_count);
    println!("  Total updates: {}", update_count);
    println!(
        "  Average rate: {:.1} updates/sec",
        update_count as f64 / start.elapsed().as_secs_f64()
    );

    Ok(())
}
