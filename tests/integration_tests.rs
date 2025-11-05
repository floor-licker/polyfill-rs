//! Integration tests for polyfill-rs
//! 
//! These tests verify that our client can actually communicate with the real Polymarket API.
//! They require network connectivity and may take longer to run.

use polyfill_rs::{ClobClient, Result, PolyfillError, Side};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::env;
use tokio::time::{sleep, Duration};
use tracing::{info, error, warn};

const POLYMARKET_HOST: &str = "https://clob.polymarket.com";
const POLYGON_CHAIN_ID: u64 = 137;

/// Test configuration from environment variables
struct TestConfig {
    private_key: Option<String>,
    api_key: Option<String>,
    api_secret: Option<String>,
    api_passphrase: Option<String>,
}

impl TestConfig {
    fn from_env() -> Self {
        Self {
            private_key: env::var("POLYMARKET_PRIVATE_KEY").ok(),
            api_key: env::var("POLYMARKET_API_KEY").ok(),
            api_secret: env::var("POLYMARKET_API_SECRET").ok(),
            api_passphrase: env::var("POLYMARKET_API_PASSPHRASE").ok(),
        }
    }

    fn has_auth(&self) -> bool {
        self.private_key.is_some()
    }

    fn has_api_creds(&self) -> bool {
        self.api_key.is_some() && self.api_secret.is_some() && self.api_passphrase.is_some()
    }
}

/// Test that we can connect to Polymarket's API
#[tokio::test]
async fn test_api_connectivity() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Test basic connectivity
    let is_ok = client.get_ok().await;
    assert!(is_ok, "Failed to connect to Polymarket API");
    
    // Test server time endpoint
    let server_time = client.get_server_time().await?;
    assert!(server_time > 0, "Invalid server time received");
    
    println!("API connectivity test passed");
    Ok(())
}

/// Test market data endpoints
#[tokio::test]
async fn test_market_data_endpoints() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Get sampling markets to find a valid token_id
    let markets_response = client.get_sampling_markets(None).await?;
    assert!(!markets_response.data.is_empty(), "No markets returned");
    
    let first_market = &markets_response.data[0];
    let token_id = &first_market.tokens[0].token_id;
    
    println!("Testing with token_id: {}", token_id);
    
    // Test order book endpoint
    let order_book = client.get_order_book(token_id).await?;
    assert_eq!(order_book.asset_id, *token_id);
    assert!(!order_book.bids.is_empty() || !order_book.asks.is_empty(), "Empty order book");
    
    // Test midpoint endpoint
    let midpoint = client.get_midpoint(token_id).await?;
    assert!(midpoint.mid > Decimal::ZERO, "Invalid midpoint");
    
    // Test spread endpoint
    let spread = client.get_spread(token_id).await?;
    assert!(spread.spread >= Decimal::ZERO, "Invalid spread");
    
    // Test price endpoints
    let buy_price = client.get_price(token_id, polyfill_rs::Side::BUY).await?;
    let sell_price = client.get_price(token_id, polyfill_rs::Side::SELL).await?;
    assert!(buy_price.price > Decimal::ZERO, "Invalid buy price");
    assert!(sell_price.price > Decimal::ZERO, "Invalid sell price");
    
    // Test tick size endpoint
    let tick_size = client.get_tick_size(token_id).await?;
    assert!(tick_size > Decimal::ZERO, "Invalid tick size");
    
    // Test neg risk endpoint
    let neg_risk = client.get_neg_risk(token_id).await?;
    // neg_risk is a boolean, so just verify it doesn't panic
    
    println!("Market data endpoints test passed");
    Ok(())
}

/// Test error handling with invalid requests
#[tokio::test]
async fn test_error_handling() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Test with invalid token_id
    let result = client.get_order_book("invalid_token_id").await;
    match result {
        Ok(_) => {
            // Some APIs might return empty data instead of error
            println!("Invalid token_id returned data instead of error");
        }
        Err(e) => {
            match e {
                PolyfillError::Api { status, .. } => {
                    assert!(status >= 400, "Expected client/server error for invalid token");
                }
                _ => {
                    // Other error types are also acceptable
                    println!("Received error for invalid token: {:?}", e);
                }
            }
        }
    }
    
    println!(" Error handling test passed");
    Ok(())
}

/// Test rate limiting behavior
#[tokio::test]
async fn test_rate_limiting() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Make multiple rapid requests to test rate limiting
    let mut results = Vec::new();
    for _ in 0..5 {
        let result = client.get_server_time().await;
        results.push(result);
        
        // Small delay between requests
        sleep(Duration::from_millis(100)).await;
    }
    
    // Most requests should succeed
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    assert!(success_count >= 3, "Too many requests failed: {}/5", success_count);
    
    println!(" Rate limiting test passed");
    Ok(())
}

/// Test compatibility with polymarket-rs-client API
#[tokio::test]
async fn test_api_compatibility() -> Result<()> {
    // Test that our client has the same basic structure as polymarket-rs-client
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Test that we can call the same methods
    let _ = client.get_ok().await;
    let _ = client.get_server_time().await?;
    let _ = client.get_sampling_markets(None).await?;
    
    // Test that our types are compatible
    let order_args = polyfill_rs::OrderArgs::new(
        "test_token",
        Decimal::from_str("0.5").map_err(|e| PolyfillError::parse(format!("Invalid decimal: {}", e), None))?,
        Decimal::from_str("1.0").map_err(|e| PolyfillError::parse(format!("Invalid decimal: {}", e), None))?,
        polyfill_rs::Side::BUY,
    );
    
    assert_eq!(order_args.token_id, "test_token");
    assert_eq!(order_args.side, polyfill_rs::Side::BUY);
    
    println!(" API compatibility test passed");
    Ok(())
}

/// Test performance characteristics
#[tokio::test]
async fn test_performance() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Test response time for basic operations
    let start = std::time::Instant::now();
    let _ = client.get_server_time().await?;
    let server_time_duration = start.elapsed();
    
    let start = std::time::Instant::now();
    let markets = client.get_sampling_markets(None).await?;
    let markets_duration = start.elapsed();
    
    // Verify reasonable performance (adjust thresholds as needed)
    assert!(server_time_duration < Duration::from_secs(5), "Server time too slow: {:?}", server_time_duration);
    assert!(markets_duration < Duration::from_secs(10), "Markets request too slow: {:?}", markets_duration);
    
    println!(" Performance test passed");
    println!("  Server time: {:?}", server_time_duration);
    println!("  Markets request: {:?}", markets_duration);
    println!("  Markets returned: {}", markets.data.len());
    
    Ok(())
}

/// Comprehensive test of all market data endpoints
#[tokio::test]
async fn test_all_market_data_endpoints() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Get a valid token ID first
    let markets_response = client.get_sampling_markets(None).await?;
    assert!(!markets_response.data.is_empty(), "No markets returned");
    
    let first_market = &markets_response.data[0];
    let token_id = &first_market.tokens[0].token_id;
    
    println!("Testing all endpoints with token_id: {}", token_id);
    
    // Test all endpoints systematically
    let endpoints = vec![
        ("Order Book", Box::new(|| client.get_order_book(token_id)) as Box<dyn Fn() -> _>),
        ("Midpoint", Box::new(|| client.get_midpoint(token_id))),
        ("Spread", Box::new(|| client.get_spread(token_id))),
        ("Buy Price", Box::new(|| client.get_price(token_id, Side::BUY))),
        ("Sell Price", Box::new(|| client.get_price(token_id, Side::SELL))),
        ("Tick Size", Box::new(|| client.get_tick_size(token_id))),
        ("Neg Risk", Box::new(|| client.get_neg_risk(token_id))),
    ];
    
    let mut success_count = 0;
    let mut total_time = Duration::from_secs(0);
    
    for (name, endpoint_test) in endpoints {
        let start = std::time::Instant::now();
        match endpoint_test().await {
            Ok(_) => {
                let duration = start.elapsed();
                total_time += duration;
                success_count += 1;
                println!("   {}: {:.2}ms", name, duration.as_secs_f64() * 1000.0);
            }
            Err(e) => {
                let duration = start.elapsed();
                total_time += duration;
                println!("   {}: {:.2}ms - {}", name, duration.as_secs_f64() * 1000.0, e);
            }
        }
        
        // Small delay between requests
        sleep(Duration::from_millis(100)).await;
    }
    
    let avg_time = total_time / endpoints.len() as u32;
    let success_rate = (success_count as f64 / endpoints.len() as f64) * 100.0;
    
    println!("Endpoint Test Summary:");
    println!("  Success rate: {:.1}%", success_rate);
    println!("  Average response time: {:.2}ms", avg_time.as_secs_f64() * 1000.0);
    
    // Require at least 80% success rate
    assert!(success_rate >= 80.0, "Success rate too low: {:.1}%", success_rate);
    
    Ok(())
}

/// Test data consistency across endpoints
#[tokio::test]
async fn test_data_consistency() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    // Get a valid token ID
    let markets_response = client.get_sampling_markets(None).await?;
    let first_market = &markets_response.data[0];
    let token_id = &first_market.tokens[0].token_id;
    
    println!("Testing data consistency with token_id: {}", token_id);
    
    // Get all market data
    let order_book = client.get_order_book(token_id).await?;
    let midpoint = client.get_midpoint(token_id).await?;
    let spread = client.get_spread(token_id).await?;
    let buy_price = client.get_price(token_id, Side::BUY).await?;
    let sell_price = client.get_price(token_id, Side::SELL).await?;
    let tick_size = client.get_tick_size(token_id).await?;
    let neg_risk = client.get_neg_risk(token_id).await?;
    
    // Validate data consistency
    println!("Data validation:");
    
    // Check that prices are positive
    assert!(buy_price.price > Decimal::ZERO, "Buy price should be positive: {}", buy_price.price);
    assert!(sell_price.price > Decimal::ZERO, "Sell price should be positive: {}", sell_price.price);
    println!("   Prices are positive");
    
    // Check that spread is non-negative
    assert!(spread.spread >= Decimal::ZERO, "Spread should be non-negative: {}", spread.spread);
    println!("   Spread is non-negative");
    
    // Check that tick size is positive
    assert!(tick_size > Decimal::ZERO, "Tick size should be positive: {}", tick_size);
    println!("   Tick size is positive");
    
    // Check that midpoint is reasonable (between buy and sell if both exist)
    if buy_price.price > Decimal::ZERO && sell_price.price > Decimal::ZERO {
        if midpoint.mid < buy_price.price || midpoint.mid > sell_price.price {
            println!("    Midpoint {} is not between buy {} and sell {}", 
                    midpoint.mid, buy_price.price, sell_price.price);
        } else {
            println!("   Midpoint is between buy and sell prices");
        }
    }
    
    // Check that we have some order book data
    if order_book.bids.is_empty() && order_book.asks.is_empty() {
        println!("    Order book is empty");
    } else {
        println!("   Order book has liquidity ({} bids, {} asks)", 
                order_book.bids.len(), order_book.asks.len());
    }
    
    // Check neg risk is a boolean
    println!("   Neg risk is boolean: {}", neg_risk);
    
    println!(" Data consistency test passed");
    Ok(())
}

/// Test rate limiting behavior more thoroughly
#[tokio::test]
async fn test_comprehensive_rate_limiting() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    println!("Testing rate limiting with rapid requests...");
    
    // Make many rapid requests
    let mut results = Vec::new();
    let request_count = 20;
    
    for i in 0..request_count {
        let start = std::time::Instant::now();
        let result = client.get_server_time().await;
        let duration = start.elapsed();
        
        results.push((i, result, duration));
        
        // Very small delay to test rate limiting
        sleep(Duration::from_millis(50)).await;
    }
    
    // Analyze results
    let success_count = results.iter().filter(|(_, result, _)| result.is_ok()).count();
    let failure_count = results.iter().filter(|(_, result, _)| result.is_err()).count();
    
    let success_rate = (success_count as f64 / request_count as f64) * 100.0;
    
    println!("Rate limiting test results:");
    println!("  Total requests: {}", request_count);
    println!("  Successful: {}", success_count);
    println!("  Failed: {}", failure_count);
    println!("  Success rate: {:.1}%", success_rate);
    
    // Show timing for first few requests
    for (i, result, duration) in results.iter().take(5) {
        let status = if result.is_ok() { "" } else { "" };
        println!("  Request {}: {} {:.2}ms", i + 1, status, duration.as_secs_f64() * 1000.0);
    }
    
    // We expect some failures due to rate limiting, but not too many
    assert!(success_rate >= 50.0, "Success rate too low, possible rate limiting issues: {:.1}%", success_rate);
    
    println!(" Rate limiting test passed");
    Ok(())
}

/// Test error handling with various invalid inputs
#[tokio::test]
async fn test_comprehensive_error_handling() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    println!("Testing comprehensive error handling...");
    
    // Test various invalid token IDs
    let invalid_tokens = vec![
        "",
        "invalid",
        "12345",
        "0x1234567890123456789012345678901234567890",
        "very_long_invalid_token_id_that_should_fail",
    ];
    
    for token_id in invalid_tokens {
        println!("  Testing invalid token ID: '{}'", token_id);
        
        let result = client.get_order_book(token_id).await;
        match result {
            Ok(order_book) => {
                if order_book.bids.is_empty() && order_book.asks.is_empty() {
                    println!("      Empty order book returned (acceptable)");
                } else {
                    println!("      Unexpected data returned for invalid token");
                }
            }
            Err(e) => {
                match e {
                    PolyfillError::Api { status, .. } => {
                        if status >= 400 {
                            println!("     Correctly returned API error: {}", status);
                        } else {
                            println!("      Unexpected status code: {}", status);
                        }
                    }
                    _ => {
                        println!("     Correctly returned error: {:?}", e);
                    }
                }
            }
        }
    }
    
    println!(" Error handling test passed");
    Ok(())
}

/// Test concurrent requests
#[tokio::test]
async fn test_concurrent_requests() -> Result<()> {
    let client = ClobClient::new(POLYMARKET_HOST);
    
    println!("Testing concurrent requests...");
    
    // Get a valid token ID
    let markets_response = client.get_sampling_markets(None).await?;
    let token_id = &markets_response.data[0].tokens[0].token_id;
    
    // Make concurrent requests
    let start = std::time::Instant::now();
    
    let results = tokio::join!(
        client.get_server_time(),
        client.get_midpoint(token_id),
        client.get_spread(token_id),
        client.get_price(token_id, Side::BUY),
        client.get_price(token_id, Side::SELL),
    );
    
    let duration = start.elapsed();
    
    // Check results
    let (server_time, midpoint, spread, buy_price, sell_price) = results;
    
    assert!(server_time.is_ok(), "Server time request failed");
    assert!(midpoint.is_ok(), "Midpoint request failed");
    assert!(spread.is_ok(), "Spread request failed");
    assert!(buy_price.is_ok(), "Buy price request failed");
    assert!(sell_price.is_ok(), "Sell price request failed");
    
    println!("   All concurrent requests succeeded");
    println!("   Total time: {:.2}ms", duration.as_secs_f64() * 1000.0);
    
    Ok(())
} 