use polyfill_rs::ClobClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("📦 Payload Size Comparison");
    println!("==========================\n");

    let api_key = std::env::var("POLYMARKET_API_KEY")?;
    let secret = std::env::var("POLYMARKET_SECRET")?;
    let passphrase = std::env::var("POLYMARKET_PASSPHRASE")?;

    let api_creds = polyfill_rs::ApiCredentials {
        api_key,
        secret,
        passphrase,
    };

    let mut client = ClobClient::new("https://clob.polymarket.com");
    client.set_api_creds(api_creds);

    println!("Testing different endpoints and parameters...\n");

    // Test 1: sampling-markets (what we're currently using)
    let response1 = client.http_client
        .get(format!("{}/sampling-markets?next_cursor=MA==", client.base_url))
        .send()
        .await?;
    
    let body1 = response1.bytes().await?;
    let json1: serde_json::Value = serde_json::from_slice(&body1)?;
    let count1 = json1["data"].as_array().map(|a| a.len()).unwrap_or(0);
    
    println!("1. /sampling-markets (default):");
    println!("   Response: {} bytes", body1.len());
    println!("   Markets:  {}", count1);
    println!();

    // Test 2: markets endpoint
    let response2 = client.http_client
        .get(format!("{}/markets", client.base_url))
        .send()
        .await?;
    
    let body2 = response2.bytes().await?;
    let json2: serde_json::Value = serde_json::from_slice(&body2)?;
    let count2 = json2["data"].as_array().map(|a| a.len()).unwrap_or(0);
    
    println!("2. /markets (no cursor):");
    println!("   Response: {} bytes", body2.len());
    println!("   Markets:  {}", count2);
    println!();

    // Test 3: simplified-markets
    let response3 = client.http_client
        .get(format!("{}/simplified-markets?next_cursor=MA==", client.base_url))
        .send()
        .await?;
    
    let body3 = response3.bytes().await?;
    let json3: serde_json::Value = serde_json::from_slice(&body3)?;
    let count3 = json3["data"].as_array().map(|a| a.len()).unwrap_or(0);
    
    println!("3. /simplified-markets:");
    println!("   Response: {} bytes", body3.len());
    println!("   Markets:  {}", count3);
    println!();

    println!("💡 Analysis:");
    println!("============");
    println!("The polymarket-rs-client might be:");
    println!("1. Using a different endpoint with less data");
    println!("2. Requesting fewer markets (pagination)");
    println!("3. Using HTTP/2 multiplexing for better performance");
    println!("4. Making fewer redundant requests");
    println!();
    println!("📌 Recommendation:");
    println!("For typical use cases, consider:");
    println!("- Using /simplified-markets for listings (smaller payload)");
    println!("- Adding limit parameters to reduce payload");
    println!("- Caching market data locally");

    Ok(())
}
