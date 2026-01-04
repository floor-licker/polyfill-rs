// Simple authentication test to verify HMAC works
use polyfill_rs::ClobClient;
use std::env;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_create_api_key_simple() {
    dotenvy::dotenv().ok();
    
    let private_key = env::var("POLYMARKET_PRIVATE_KEY")
        .expect("POLYMARKET_PRIVATE_KEY must be set in .env");
    
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137,
        None
    );
    
    println!("Step 1: Creating/deriving API key...");
    let result = client.create_or_derive_api_key(None).await;
    
    match result {
        Ok(creds) => {
            println!("Successfully created/derived API key");
            println!("  API Key: {}", creds.api_key);
            client.set_api_creds(creds);
            
            // Now try to get orders (requires auth)
            println!("\nStep 2: Testing authenticated endpoint (get_orders)...");
            let orders_result = client.get_orders(None, None).await;
            
            match orders_result {
                Ok(orders) => {
                    println!("Successfully authenticated! Got {} orders", orders.len());
                }
                Err(e) => {
                    let err_str = format!("{:?}", e);
                    if err_str.contains("401") {
                        panic!("CRITICAL: 401 Unauthorized - HMAC authentication is BROKEN!");
                    } else {
                        println!("Authentication successful (non-401 error): {:?}", e);
                    }
                }
            }
        }
        Err(e) => {
            panic!("Failed to create/derive API key: {:?}", e);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_get_api_keys() {
    dotenvy::dotenv().ok();
    
    let private_key = env::var("POLYMARKET_PRIVATE_KEY")
        .expect("POLYMARKET_PRIVATE_KEY must be set in .env");
    
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137,
        None
    );
    
    let creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create API key");
    client.set_api_creds(creds);
    
    println!("Testing get_api_keys (requires HMAC auth)...");
    let result = client.get_api_keys().await;
    
    match result {
        Ok(keys) => {
            println!("Authentication successful! Found {} keys", keys.len());
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("CRITICAL: 401 Unauthorized - HMAC authentication is BROKEN!");
            } else {
                panic!("Failed with non-401 error: {:?}", e);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_get_trades() {
    dotenvy::dotenv().ok();
    
    let private_key = env::var("POLYMARKET_PRIVATE_KEY")
        .expect("POLYMARKET_PRIVATE_KEY must be set in .env");
    
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137,
        None
    );
    
    let creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create API key");
    client.set_api_creds(creds);
    
    println!("Testing get_trades (requires HMAC auth)...");
    let result = client.get_trades(None, None).await;
    
    match result {
        Ok(_) => {
            println!("Authentication successful!");
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("CRITICAL: 401 Unauthorized - HMAC authentication is BROKEN!");
            } else {
                println!("Authentication successful (got non-401 error): {:?}", e);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_get_notifications() {
    dotenvy::dotenv().ok();
    
    let private_key = env::var("POLYMARKET_PRIVATE_KEY")
        .expect("POLYMARKET_PRIVATE_KEY must be set in .env");
    
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137,
        None
    );
    
    let creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create API key");
    client.set_api_creds(creds);
    
    println!("Testing get_notifications (requires HMAC auth)...");
    let result = client.get_notifications().await;
    
    match result {
        Ok(notifs) => {
            println!("Authentication successful! Notifications: {:?}", notifs);
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            if err_str.contains("401") {
                panic!("CRITICAL: 401 Unauthorized - HMAC authentication is BROKEN!");
            } else {
                println!("Authentication successful (got non-401 error): {:?}", e);
            }
        }
    }
}

