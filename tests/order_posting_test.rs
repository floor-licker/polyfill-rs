// Test order posting - the critical endpoint that had the 401 bug
use polyfill_rs::{ClobClient, OrderArgs, Side};
use rust_decimal::Decimal;
use std::env;
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_post_order_authentication() {
    dotenvy::dotenv().ok();
    
    let private_key = env::var("POLYMARKET_PRIVATE_KEY")
        .expect("POLYMARKET_PRIVATE_KEY must be set in .env");
    
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137
    );
    
    println!("Step 1: Creating API credentials...");
    let creds = client.create_or_derive_api_key(None).await
        .expect("Failed to create API key");
    client.set_api_creds(creds);
    println!("API credentials set");
    
    // Use a well-known token ID (we'll use an extreme price so it won't fill)
    let token_id = "21742633143463906290569050155826241533067272736897614950488156847949938836455"; // Example token
    
    println!("\nStep 2: Attempting to post order (testing authentication)...");
    let order_args = OrderArgs {
        token_id: token_id.to_string(),
        price: Decimal::from_str("0.01").unwrap(), // Very low price, won't fill
        size: Decimal::from_str("1.0").unwrap(),
        side: Side::BUY,
    };
    
    let result = client.create_and_post_order(&order_args).await;
    
    match result {
        Ok(response) => {
            println!("AUTHENTICATION SUCCESSFUL! Order was accepted by API");
            println!("  Response: {:?}", response);
            
            // Try to cancel it if we got an order ID
            if let Some(order_id) = response.get("orderID").and_then(|v| v.as_str()) {
                println!("\nStep 3: Canceling order...");
                match client.cancel(order_id).await {
                    Ok(_) => println!("Order canceled successfully"),
                    Err(e) => println!("Cancel failed (order might have expired): {:?}", e),
                }
            }
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            
            // The critical test: Is it a 401 error?
            if err_str.contains("401") {
                panic!("CRITICAL FAILURE: 401 Unauthorized!\n\
                        The HMAC authentication bug is NOT fixed!\n\
                        Error: {:?}", e);
            }
            
            // If it's a 400 error with validation issues, that's actually GOOD
            // It means authentication worked, but there's an issue with the order parameters
            if err_str.contains("400") {
                println!("AUTHENTICATION SUCCESSFUL!");
                println!("  (Got 400 validation error, which means auth passed)");
                println!("  Error details: {}", err_str);
                
                // These are expected validation errors when auth works
                if err_str.contains("insufficient") || 
                   err_str.contains("balance") ||
                   err_str.contains("allowance") ||
                   err_str.contains("POLY_AMOUNT_TOO_SMALL") ||
                   err_str.contains("invalid") ||
                   err_str.contains("market") {
                    println!("  This is an expected validation error - authentication is working!");
                    return;
                }
            }
            
            // Any other error type
            println!("Got unexpected error (not 401, so auth might be OK): {:?}", e);
        }
    }
}

