//! Quick Demo for polyfill-rs
//!
//! This example demonstrates all available API endpoints in a simple, easy-to-run format.
//! It can be run without authentication credentials and will test all public endpoints.

use polyfill_rs::{ClobClient, PolyfillError, Result, Side};
use rust_decimal::Decimal;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

/// Quick demo that tests all available endpoints
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Polyfill-rs Quick Demo");
    info!("======================");

    // Create client
    let client = ClobClient::new("https://clob.polymarket.com");

    // Test 1: Basic connectivity
    info!("\nTesting API Connectivity...");
    match test_connectivity(&client).await {
        Ok(_) => info!("API connectivity test passed"),
        Err(e) => {
            error!("API connectivity test failed: {}", e);
            return Err(e);
        },
    }

    // Test 2: Get a valid token ID from markets
    info!("\nGetting Market Data...");
    let token_id = match get_valid_token_id(&client).await {
        Ok(id) => {
            info!("Found valid token ID: {}", id);
            id
        },
        Err(e) => {
            error!("Failed to get valid token ID: {}", e);
            return Err(e);
        },
    };

    // Test 3: Test all market data endpoints
    info!("\nTesting Market Data Endpoints...");
    test_market_data_endpoints(&client, &token_id).await?;

    // Test 4: Test error handling
    info!("\nTesting Error Handling...");
    test_error_handling(&client).await?;

    // Test 5: Performance test
    info!("\nTesting Performance...");
    test_performance(&client, &token_id).await?;

    info!("\nAll tests completed successfully!");
    info!("The polyfill-rs client is working correctly with the Polymarket API.");

    Ok(())
}

/// Test basic API connectivity
async fn test_connectivity(client: &ClobClient) -> Result<()> {
    // Test /ok endpoint
    let is_ok = client.get_ok().await;
    if !is_ok {
        return Err(PolyfillError::network(
            "API not responding",
            std::io::Error::other("API not responding"),
        ));
    }
    info!("  /ok endpoint responding");

    // Test /time endpoint
    let server_time = client.get_server_time().await?;
    info!("  Server time: {}", server_time);

    // Verify server time is reasonable (within last 24 hours)
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let time_diff = server_time.abs_diff(current_time);

    if time_diff > 86400 {
        // 24 hours
        warn!("  Server time seems off (diff: {} seconds)", time_diff);
    } else {
        info!("  Server time is reasonable");
    }

    Ok(())
}

/// Get a valid token ID from the markets endpoint
async fn get_valid_token_id(client: &ClobClient) -> Result<String> {
    let markets = client.get_sampling_markets(None).await?;

    if markets.data.is_empty() {
        return Err(PolyfillError::api(404, "No markets found"));
    }

    // Find a market with active tokens
    for market in &markets.data {
        if market.active && !market.closed {
            for token in &market.tokens {
                if !token.token_id.is_empty() {
                    info!("  Found active market: {}", market.question);
                    info!("  Market slug: {}", market.market_slug);
                    info!("  Token ID: {}", token.token_id);
                    info!("  Outcome: {}", token.outcome);
                    return Ok(token.token_id.clone());
                }
            }
        }
    }

    Err(PolyfillError::api(
        404,
        "No active markets with valid tokens found",
    ))
}

/// Test all market data endpoints
async fn test_market_data_endpoints(client: &ClobClient, token_id: &str) -> Result<()> {
    // Test order book
    info!("  Testing order book endpoint...");
    let order_book = client.get_order_book(token_id).await?;
    info!(
        "    Order book: {} bids, {} asks",
        order_book.bids.len(),
        order_book.asks.len()
    );

    // Test midpoint
    info!("  Testing midpoint endpoint...");
    let midpoint = client.get_midpoint(token_id).await?;
    info!("    Midpoint: {}", midpoint.mid);

    // Test spread
    info!("  Testing spread endpoint...");
    let spread = client.get_spread(token_id).await?;
    info!("    Spread: {}", spread.spread);

    // Test buy price
    info!("  Testing buy price endpoint...");
    let buy_price = client.get_price(token_id, Side::BUY).await?;
    info!("    Buy price: {}", buy_price.price);

    // Test sell price
    info!("  Testing sell price endpoint...");
    let sell_price = client.get_price(token_id, Side::SELL).await?;
    info!("    Sell price: {}", sell_price.price);

    // Test tick size
    info!("  Testing tick size endpoint...");
    let tick_size = client.get_tick_size(token_id).await?;
    info!("    Tick size: {}", tick_size);

    // Test neg risk
    info!("  Testing neg risk endpoint...");
    let neg_risk = client.get_neg_risk(token_id).await?;
    info!("    Neg risk: {}", neg_risk);

    // Validate data consistency
    info!("  Validating data consistency...");
    validate_market_data(&order_book, &midpoint, &spread, &buy_price, &sell_price)?;

    Ok(())
}

/// Validate that market data is consistent
fn validate_market_data(
    order_book: &polyfill_rs::client::OrderBookSummary,
    midpoint: &polyfill_rs::client::MidpointResponse,
    spread: &polyfill_rs::client::SpreadResponse,
    buy_price: &polyfill_rs::client::PriceResponse,
    sell_price: &polyfill_rs::client::PriceResponse,
) -> Result<()> {
    // Check that we have some liquidity
    if order_book.bids.is_empty() && order_book.asks.is_empty() {
        warn!("    Order book is empty");
    } else {
        info!("    Order book has liquidity");
    }

    // Check that prices are positive
    if buy_price.price <= Decimal::ZERO {
        warn!("    Buy price is not positive: {}", buy_price.price);
    } else {
        info!("    Buy price is positive");
    }

    if sell_price.price <= Decimal::ZERO {
        warn!("    Sell price is not positive: {}", sell_price.price);
    } else {
        info!("    Sell price is positive");
    }

    // Check that spread is reasonable
    if spread.spread < Decimal::ZERO {
        warn!("    Spread is negative: {}", spread.spread);
    } else {
        info!("    Spread is non-negative");
    }

    // Check that midpoint is between buy and sell prices (if both exist)
    if buy_price.price > Decimal::ZERO && sell_price.price > Decimal::ZERO {
        if midpoint.mid < buy_price.price || midpoint.mid > sell_price.price {
            warn!(
                "    Midpoint {} is not between buy {} and sell {}",
                midpoint.mid, buy_price.price, sell_price.price
            );
        } else {
            info!("    Midpoint is between buy and sell prices");
        }
    }

    Ok(())
}

/// Test error handling with invalid requests
async fn test_error_handling(client: &ClobClient) -> Result<()> {
    // Test with invalid token ID
    info!("  Testing invalid token ID...");
    let result = client.get_order_book("invalid_token_12345").await;
    match result {
        Ok(_) => {
            warn!("    Invalid token ID returned data instead of error");
        },
        Err(e) => match e {
            PolyfillError::Api { status, .. } => {
                if status >= 400 {
                    info!("    Invalid token ID correctly returned error: {}", status);
                } else {
                    warn!("    Unexpected status code for invalid token: {}", status);
                }
            },
            _ => {
                info!("    Invalid token ID returned error: {:?}", e);
            },
        },
    }

    // Test with empty token ID
    info!("  Testing empty token ID...");
    let result = client.get_order_book("").await;
    match result {
        Ok(_) => {
            warn!("    Empty token ID returned data instead of error");
        },
        Err(e) => {
            info!("    Empty token ID correctly returned error: {:?}", e);
        },
    }

    Ok(())
}

/// Test performance characteristics
async fn test_performance(client: &ClobClient, token_id: &str) -> Result<()> {
    let mut total_time = Duration::from_secs(0);
    let mut success_count = 0;
    let test_count = 5;

    info!("  Running {} performance tests...", test_count);

    for i in 1..=test_count {
        let start = std::time::Instant::now();

        // Test a mix of endpoints
        let results = tokio::join!(
            client.get_server_time(),
            client.get_midpoint(token_id),
            client.get_spread(token_id),
        );

        let duration = start.elapsed();
        total_time += duration;

        match results {
            (Ok(_), Ok(_), Ok(_)) => {
                success_count += 1;
                info!(
                    "    Test {}: PASSED {:.2}ms",
                    i,
                    duration.as_secs_f64() * 1000.0
                );
            },
            _ => {
                warn!(
                    "    Test {}: FAILED in {:.2}ms",
                    i,
                    duration.as_secs_f64() * 1000.0
                );
            },
        }

        // Small delay between tests
        sleep(Duration::from_millis(100)).await;
    }

    let avg_time = total_time / test_count as u32;
    let success_rate = (success_count as f64 / test_count as f64) * 100.0;

    info!("  Performance Summary:");
    info!("    Success rate: {:.1}%", success_rate);
    info!(
        "    Average response time: {:.2}ms",
        avg_time.as_secs_f64() * 1000.0
    );
    info!("    Total time: {:.2}s", total_time.as_secs_f64());

    // Performance thresholds
    if avg_time > Duration::from_secs(2) {
        warn!(
            "    Average response time is slow: {:.2}ms",
            avg_time.as_secs_f64() * 1000.0
        );
    } else {
        info!("    Response times are acceptable");
    }

    if success_rate < 80.0 {
        warn!("    Success rate is low: {:.1}%", success_rate);
    } else {
        info!("    Success rate is good");
    }

    Ok(())
}
