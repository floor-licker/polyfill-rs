use polyfill_rs::ClobClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file
    dotenv::dotenv().ok();
    
    println!("🔧 Environment Configuration Example");
    println!("===================================");
    
    // Check if credentials are available
    match (
        std::env::var("POLYMARKET_API_KEY"),
        std::env::var("POLYMARKET_SECRET"),
        std::env::var("POLYMARKET_PASSPHRASE"),
    ) {
        (Ok(api_key), Ok(secret), Ok(passphrase)) => {
            println!("✅ All credentials loaded from .env file:");
            println!("   API Key: {}...{}", &api_key[..8], &api_key[api_key.len()-8..]);
            println!("   Secret: {}...{}", &secret[..8], &secret[secret.len()-8..]);
            println!("   Passphrase: {}...{}", &passphrase[..8], &passphrase[passphrase.len()-8..]);
            
            // Create client (would normally use these credentials)
            let client = ClobClient::new_internet("https://clob.polymarket.com");
            
            // Test basic connectivity
            println!("\n🌐 Testing connectivity...");
            match client.get_server_time().await {
                Ok(timestamp) => {
                    println!("✅ Server time: {}", timestamp);
                    println!("🚀 Ready for authenticated operations!");
                }
                Err(e) => {
                    println!("❌ Connection error: {}", e);
                }
            }
        }
        _ => {
            println!("❌ Missing credentials in .env file");
            println!("📝 Create a .env file with:");
            println!("   POLYMARKET_API_KEY=your-api-key");
            println!("   POLYMARKET_SECRET=your-secret");
            println!("   POLYMARKET_PASSPHRASE=your-passphrase");
        }
    }
    
    Ok(())
}
