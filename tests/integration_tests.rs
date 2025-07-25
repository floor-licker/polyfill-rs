//! Integration tests for polyfill-rs
//! 
//! These tests verify that our client can actually communicate with the real Polymarket API.
//! They require network connectivity and may take longer to run.

use polyfill_rs::{ClobClient, Result, PolyfillError};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::env;
use tokio::time::{sleep, Duration};

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
    
    println!("✅ API connectivity test passed");
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
    
    println!("✅ Market data endpoints test passed");
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
            println!("⚠️  Invalid token_id returned data instead of error");
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
    
    println!("✅ Error handling test passed");
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
    
    println!("✅ Rate limiting test passed");
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
    
    println!("✅ API compatibility test passed");
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
    
    println!("✅ Performance test passed");
    println!("  Server time: {:?}", server_time_duration);
    println!("  Markets request: {:?}", markets_duration);
    println!("  Markets returned: {}", markets.data.len());
    
    Ok(())
} 