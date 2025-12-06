use polyfill_rs::ClobClient;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("🔍 Detailed Latency Profiling");
    println!("=============================\n");

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

    println!("Running 5 requests with detailed timing breakdown...\n");

    for i in 1..=5 {
        println!("Request {}:", i);
        
        let total_start = Instant::now();
        
        // DNS + Connection establishment
        let connect_start = Instant::now();
        let response = client.http_client
            .get(format!("{}/sampling-markets?next_cursor=MA==", client.base_url))
            .send()
            .await?;
        let connect_time = connect_start.elapsed();
        
        // Response headers received
        let status = response.status();
        let headers_time = connect_start.elapsed();
        
        // Read response body
        let body_start = Instant::now();
        let body_bytes = response.bytes().await?;
        let body_time = body_start.elapsed();
        
        // Parse JSON
        let parse_start = Instant::now();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes)?;
        let parse_time = parse_start.elapsed();
        
        let total_time = total_start.elapsed();
        
        // Calculate derived metrics
        let network_time = connect_time;
        let download_time = body_time;
        let overhead = total_time.saturating_sub(network_time + download_time + parse_time);
        
        println!("  Total:       {:>8.1} ms", total_time.as_micros() as f64 / 1000.0);
        println!("  Network:     {:>8.1} ms  (DNS + TCP + TLS + HTTP)", network_time.as_micros() as f64 / 1000.0);
        println!("  Headers:     {:>8.1} ms  (time to first byte)", headers_time.as_micros() as f64 / 1000.0);
        println!("  Download:    {:>8.1} ms  (response body)", download_time.as_micros() as f64 / 1000.0);
        println!("  JSON Parse:  {:>8.1} ms  (deserialization)", parse_time.as_micros() as f64 / 1000.0);
        println!("  Overhead:    {:>8.1} ms", overhead.as_micros() as f64 / 1000.0);
        println!("  Status:      {} ({})", status.as_u16(), status.canonical_reason().unwrap_or("Unknown"));
        println!("  Body Size:   {} bytes", body_bytes.len());
        
        if let Some(markets) = json["data"].as_array() {
            println!("  Markets:     {}", markets.len());
        }
        
        println!();
        
        // Small delay between requests
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    println!("\n📊 ANALYSIS:");
    println!("============");
    println!("Network time includes:");
    println!("  - DNS resolution (if not cached)");
    println!("  - TCP connection establishment");
    println!("  - TLS handshake");
    println!("  - HTTP request/response");
    println!();
    println!("💡 OPTIMIZATION TARGETS:");
    println!("- If Network > 400ms: DNS caching, connection pooling, HTTP/2");
    println!("- If Download > 100ms: Compression, smaller payload");
    println!("- If JSON Parse > 50ms: Faster parsing, streaming parser");
    println!("- If Overhead > 50ms: Reduce allocations, optimize client");

    Ok(())
}
