use polyfill_rs::stream::WebSocketStream;
use polyfill_rs::auth::create_wss_auth;
use polyfill_rs::types::StreamMessage;
use polyfill_rs::ClobClient;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt::init();
    
    // Load private key from env
    dotenvy::from_path("../.env").ok();
    let private_key = std::env::var("PRIVATE_KEY")
        .expect("PRIVATE_KEY must be set");
    
    // Add 0x prefix if missing
    let private_key = if private_key.starts_with("0x") {
        private_key
    } else {
        format!("0x{}", private_key)
    };
    
    println!("🔌 Testing User WebSocket connection...");
    
    // Create CLOB client and derive API credentials
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137, // Polygon mainnet
    );
    
    println!("📝 Deriving API credentials...");
    let api_creds = client.create_or_derive_api_key(None).await?;
    client.set_api_creds(api_creds.clone());
    println!("✅ API credentials obtained");
    
    // Create WebSocket auth from API creds
    let wss_auth = create_wss_auth(&api_creds);
    println!("✅ WssAuth created:");
    println!("   API Key: {}...", &wss_auth.api_key[..20.min(wss_auth.api_key.len())]);
    println!("   Secret: {}...", &wss_auth.secret[..20.min(wss_auth.secret.len())]);
    println!("   Passphrase: {}", wss_auth.passphrase);
    
    // Create WebSocket stream
    let mut ws = WebSocketStream::new("wss://ws-subscriptions-clob.polymarket.com/ws/user")
        .with_auth(wss_auth);
    
    // BTC price market condition ID (15-min windows)
    let btc_market = "0x5f65177b394277fd294cd75650044e32ba009a95022ec4a738f0c3bd3d96b88b".to_string();
    
    println!("📡 Subscribing to user channel for market: {}...", &btc_market[..20]);
    
    // Subscribe to user channel
    ws.subscribe_user_channel(vec![btc_market]).await?;
    println!("✅ Subscribed! Waiting for messages (timeout: 15s)...");
    
    // Wait for messages with timeout
    let mut count = 0;
    match timeout(Duration::from_secs(15), async {
        while let Some(result) = ws.next().await {
            match result {
                Ok(msg) => {
                    count += 1;
                    match msg {
                        StreamMessage::Heartbeat { timestamp } => {
                            println!("💓 Heartbeat at {}", timestamp);
                        },
                        StreamMessage::UserOrderUpdate { data } => {
                            println!("📋 Order update: {} - {:?}", data.id, data.status);
                        },
                        StreamMessage::UserTrade { data } => {
                            println!("💰 Trade: {} {:?} @ {}", data.size, data.side, data.price);
                        },
                        other => {
                            println!("📨 Message: {:?}", other);
                        }
                    }
                    if count >= 5 {
                        println!("✅ Received 5 messages, WebSocket working!");
                        break;
                    }
                },
                Err(e) => {
                    println!("❌ Error: {}", e);
                    break;
                }
            }
        }
    }).await {
        Ok(_) => println!("✅ Test completed successfully! Got {} messages", count),
        Err(_) => {
            if count > 0 {
                println!("✅ Test completed! Got {} messages before timeout", count);
            } else {
                println!("⏰ Timeout - no messages received (this is OK if no active orders)");
            }
        },
    }
    
    println!("🔌 Disconnecting...");
    Ok(())
}
