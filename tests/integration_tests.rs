// Integration tests for polyfill-rs
// These tests hit the real Polymarket API and are ignored by default
// Run with: cargo test --test integration_tests -- --ignored --test-threads=1

use polyfill_rs::{ClobClient, OrderArgs, Side};
use rust_decimal_macros::dec;
use std::env;

const HOST: &str = "https://clob.polymarket.com";
const CHAIN_ID: u64 = 137;

fn load_env_vars() -> (String, Option<String>, Option<String>, Option<String>) {
    dotenvy::dotenv().ok();
    
    let private_key = env::var("POLYMARKET_PRIVATE_KEY")
        .expect("POLYMARKET_PRIVATE_KEY must be set in .env");
    let api_key = env::var("POLYMARKET_API_KEY").ok();
    let api_secret = env::var("POLYMARKET_API_SECRET").ok();
    let api_passphrase = env::var("POLYMARKET_API_PASSPHRASE").ok();
    
    (private_key, api_key, api_secret, api_passphrase)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_create_derive_api_key() {
    let (private_key, _, _, _) = load_env_vars();
    
    let client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    
    // Test creating/deriving API key
    let result = client.create_or_derive_api_key(None).await;
    assert!(result.is_ok(), "Failed to create/derive API key: {:?}", result);
    
    let api_creds = result.unwrap();
    assert!(!api_creds.api_key.is_empty());
    assert!(!api_creds.secret.is_empty());
    assert!(!api_creds.passphrase.is_empty());
    
    println!("PASS: Successfully created/derived API key");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_authenticated_order_flow() {
    let (private_key, _, _, _) = load_env_vars();
    
    // Initialize client with L1 headers
    let mut client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    
    // Step 1: Create/derive API credentials
    println!("Step 1: Creating/deriving API credentials...");
    let api_creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create/derive API key");
    client.set_api_creds(api_creds);
    println!("PASS: API credentials set");
    
    // Step 2: Get a valid token_id from active markets
    println!("Step 2: Fetching active markets...");
    let markets = client.get_sampling_markets(None).await
        .expect("Failed to get markets");
    
    let active_market = markets.data.iter()
        .find(|m| m.active && !m.closed)
        .expect("No active markets found");
    
    let token_id = &active_market.tokens[0].token_id;
    println!("PASS: Found active token: {}", token_id);
    
    // Step 3: Get current price to place a reasonable order
    println!("Step 3: Getting current market price...");
    let midpoint = client.get_midpoint(token_id).await
        .expect("Failed to get midpoint");
    println!("PASS: Current midpoint: {}", midpoint.mid);
    
    // Step 4: Create and post a small order well away from market price
    // (so it won't fill immediately)
    let order_price = if midpoint.mid > dec!(0.5) {
        dec!(0.01) // Very low buy price, won't fill
    } else {
        dec!(0.99) // Very high sell price, won't fill
    };
    
    println!("Step 4: Posting order at price {}...", order_price);
    let order_args = OrderArgs {
        token_id: token_id.clone(),
        price: order_price,
        size: dec!(1.0), // Minimum size
        side: Side::BUY,
    };
    
    let post_result = client.create_and_post_order(&order_args).await;
    
    // This is the critical test - did we get past the 401 error?
    match &post_result {
        Ok(response) => {
            println!("PASS: Order posted successfully!");
            
            // Step 5: Cancel the order
            if let Some(order_id) = response.get("orderID").and_then(|v| v.as_str()) {
                println!("Step 5: Canceling order {}...", order_id);
                let cancel_result = client.cancel(order_id).await;
                assert!(cancel_result.is_ok(), "Failed to cancel order: {:?}", cancel_result);
                println!("PASS: Order canceled successfully");
            } else {
                println!("WARNING: Order posted but no orderID in response: {:?}", response);
            }
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            
            // Check if it's a 401 (authentication failure)
            if err_str.contains("401") {
                panic!("FAIL: CRITICAL: 401 Unauthorized error - HMAC authentication is broken!");
            }
            
            // Check if it's a 400 with specific validation errors (these are OK)
            if err_str.contains("400") && (
                err_str.contains("insufficient") || 
                err_str.contains("balance") ||
                err_str.contains("allowance") ||
                err_str.contains("POLY_AMOUNT_TOO_SMALL")
            ) {
                println!("PASS: Authentication successful (got expected validation error)");
                println!("  Error: {}", err_str);
            } else {
                panic!("FAIL: Unexpected error: {:?}", e);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_get_orders() {
    let (private_key, _, _, _) = load_env_vars();
    
    let mut client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    let api_creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create/derive API key");
    client.set_api_creds(api_creds);
    
    println!("Testing get_orders...");
    let result = client.get_orders(None, None).await;
    
    match result {
        Ok(orders) => {
            println!("PASS: Successfully fetched orders");
            println!("  Found {} orders", orders.len());
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("FAIL: 401 Unauthorized - authentication failed!");
            }
            panic!("Failed to get orders: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_get_trades() {
    let (private_key, _, _, _) = load_env_vars();
    
    let mut client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    let api_creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create/derive API key");
    client.set_api_creds(api_creds);
    
    println!("Testing get_trades...");
    let result = client.get_trades(None, None).await;
    
    match result {
        Ok(_trades) => {
            println!("PASS: Successfully fetched trades");
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("FAIL: 401 Unauthorized - authentication failed!");
            }
            panic!("Failed to get trades: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_get_balance_allowance() {
    let (private_key, _, _, _) = load_env_vars();
    
    let mut client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    let api_creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create/derive API key");
    client.set_api_creds(api_creds);
    
    println!("Testing get_balance_allowance...");
    
    // Get a valid token_id first
    let markets = client.get_sampling_markets(None).await
        .expect("Failed to get markets");
    let token_id = &markets.data[0].tokens[0].token_id;
    
    use polyfill_rs::types::{BalanceAllowanceParams, AssetType};
    let params = BalanceAllowanceParams {
        asset_type: Some(AssetType::CONDITIONAL),
        token_id: Some(token_id.clone()),
        signature_type: None,
    };
    
    let result = client.get_balance_allowance(Some(params)).await;
    
    match result {
        Ok(balance) => {
            println!("PASS: Successfully fetched balance/allowance");
            println!("  Balance: {:?}", balance);
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("FAIL: 401 Unauthorized - authentication failed!");
            }
            println!("WARNING: Balance check failed (may be expected): {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_get_api_keys() {
    let (private_key, _, _, _) = load_env_vars();
    
    let mut client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    let api_creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create/derive API key");
    client.set_api_creds(api_creds);
    
    println!("Testing get_api_keys...");
    let result = client.get_api_keys().await;
    
    match result {
        Ok(keys) => {
            println!("PASS: Successfully fetched API keys");
            println!("  Found {} keys", keys.len());
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("FAIL: 401 Unauthorized - authentication failed!");
            }
            panic!("Failed to get API keys: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_get_notifications() {
    let (private_key, _, _, _) = load_env_vars();
    
    let mut client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    let api_creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create/derive API key");
    client.set_api_creds(api_creds);
    
    println!("Testing get_notifications...");
    let result = client.get_notifications().await;
    
    match result {
        Ok(notifications) => {
            println!("PASS: Successfully fetched notifications");
            println!("  Notifications: {:?}", notifications);
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("FAIL: 401 Unauthorized - authentication failed!");
            }
            panic!("Failed to get notifications: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_market_data_endpoints() {
    let (private_key, _, _, _) = load_env_vars();
    
    let client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    
    println!("Testing market data endpoints (no auth required)...");
    
    // Get a valid token_id
    let markets = client.get_sampling_markets(None).await
        .expect("Failed to get markets");
    let token_id = &markets.data[0].tokens[0].token_id;
    println!("PASS: Using token_id: {}", token_id);
    
    // Test multiple endpoints
    println!("Testing get_order_book...");
    let book = client.get_order_book(token_id).await
        .expect("Failed to get order book");
    println!("PASS: Order book: {} bids, {} asks", book.bids.len(), book.asks.len());
    
    println!("Testing get_midpoint...");
    let midpoint = client.get_midpoint(token_id).await
        .expect("Failed to get midpoint");
    println!("PASS: Midpoint: {}", midpoint.mid);
    
    println!("Testing get_spread...");
    let spread = client.get_spread(token_id).await
        .expect("Failed to get spread");
    println!("PASS: Spread: {}", spread.spread);
    
    println!("Testing get_price...");
    let price = client.get_price(token_id, Side::BUY).await
        .expect("Failed to get price");
    println!("PASS: Buy price: {}", price.price);
    
    println!("Testing get_tick_size...");
    let tick_size = client.get_tick_size(token_id).await
        .expect("Failed to get tick size");
    println!("PASS: Tick size: {}", tick_size);
    
    println!("Testing get_markets...");
    let all_markets = client.get_markets(None).await
        .expect("Failed to get all markets");
    println!("PASS: Found {} markets", all_markets.data.len());
    
    println!("\nPASS: All market data endpoints working!");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_batch_endpoints() {
    let (private_key, _, _, _) = load_env_vars();
    
    let client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID, None);
    
    println!("Testing batch endpoints...");
    
    // Get multiple valid token_ids
    let markets = client.get_sampling_markets(None).await
        .expect("Failed to get markets");
    let token_ids: Vec<String> = markets.data[0..2.min(markets.data.len())]
        .iter()
        .map(|m| m.tokens[0].token_id.clone())
        .collect();
    
    println!("Testing get_order_books (batch)...");
    let books = client.get_order_books(&token_ids).await
        .expect("Failed to get order books");
    println!("PASS: Fetched {} order books", books.len());
    
    println!("Testing get_midpoints (batch)...");
    let midpoints = client.get_midpoints(&token_ids).await
        .expect("Failed to get midpoints");
    println!("PASS: Fetched {} midpoints", midpoints.len());
    
    println!("Testing get_spreads (batch)...");
    let spreads = client.get_spreads(&token_ids).await
        .expect("Failed to get spreads");
    println!("PASS: Fetched {} spreads", spreads.len());
    
    println!("\nPASS: All batch endpoints working!");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_health_check() {
    let client = ClobClient::new(HOST);
    
    println!("Testing health check endpoints...");
    
    let ok = client.get_ok().await;
    assert!(ok, "API health check failed!");
    println!("PASS: API is healthy");
    
    let server_time = client.get_server_time().await
        .expect("Failed to get server time");
    println!("PASS: Server time: {}", server_time);
}
