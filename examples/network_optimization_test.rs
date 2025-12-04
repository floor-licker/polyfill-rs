use polyfill_rs::ClobClient;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Network Optimization Test - polyfill-rs");
    println!("===========================================");
    
    // Test different client configurations
    let clients = vec![
        ("Standard", ClobClient::new("https://clob.polymarket.com")),
        ("Colocated", ClobClient::new_colocated("https://clob.polymarket.com")),
        ("Internet", ClobClient::new_internet("https://clob.polymarket.com")),
    ];
    
    for (name, client) in clients {
        println!("\n📊 Testing {} Client Configuration", name);
        println!("{}=", "=".repeat(40 + name.len()));
        
        // Test 1: Server time (baseline latency)
        println!("  🔍 Server Time Test:");
        let mut times = Vec::new();
        for i in 0..10 {
            let start = Instant::now();
            match client.get_server_time().await {
                Ok(timestamp) => {
                    let duration = start.elapsed();
                    times.push(duration);
                    if i < 2 {
                        println!("    Run {}: ✅ {} in {:?}", i+1, timestamp, duration);
                    }
                }
                Err(e) => {
                    let duration = start.elapsed();
                    times.push(duration);
                    if i < 2 {
                        println!("    Run {}: ❌ Error in {:?}: {}", i+1, duration, e);
                    }
                }
            }
        }
        
        if !times.is_empty() {
            let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
            let min = times.iter().min().unwrap();
            let max = times.iter().max().unwrap();
            let std_dev = {
                let mean = avg.as_millis() as f64;
                let variance = times.iter()
                    .map(|t| (t.as_millis() as f64 - mean).powi(2))
                    .sum::<f64>() / times.len() as f64;
                variance.sqrt()
            };
            
            println!("    📈 Average: {:.1}ms ± {:.1}ms", avg.as_millis(), std_dev);
            println!("    📊 Range: {:?} - {:?}", min, max);
            println!("    🌐 Best: {:?}", min);
        }
        
        // Test 2: Market data fetching
        println!("  🔍 Market Data Test:");
        let mut times = Vec::new();
        for i in 0..5 {
            let start = Instant::now();
            match client.get_sampling_simplified_markets(None).await {
                Ok(markets) => {
                    let duration = start.elapsed();
                    times.push(duration);
                    if i < 2 {
                        println!("    Run {}: ✅ {} markets in {:?}", i+1, markets.data.len(), duration);
                    }
                }
                Err(e) => {
                    let duration = start.elapsed();
                    times.push(duration);
                    if i < 2 {
                        println!("    Run {}: ❌ Error in {:?}: {}", i+1, duration, e);
                    }
                }
            }
        }
        
        if !times.is_empty() {
            let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
            let min = times.iter().min().unwrap();
            let max = times.iter().max().unwrap();
            
            println!("    📈 Average: {:?}", avg);
            println!("    📊 Range: {:?} - {:?}", min, max);
            println!("    🌐 Best: {:?}", min);
        }
        
        // Test 3: Connection reuse test
        println!("  🔍 Connection Reuse Test:");
        let start = Instant::now();
        for i in 0..5 {
            match client.get_server_time().await {
                Ok(_) => {
                    if i == 0 {
                        println!("    First request: {:?}", start.elapsed());
                    }
                }
                Err(e) => {
                    println!("    Error on request {}: {}", i+1, e);
                    break;
                }
            }
        }
        let total_time = start.elapsed();
        println!("    📈 5 requests total: {:?}", total_time);
        println!("    📊 Average per request: {:?}", total_time / 5);
    }
    
    println!("\n🎯 Network Optimization Summary");
    println!("===============================");
    println!("HTTP Client Optimizations Applied:");
    println!("  • Connection pooling (10-20 connections per host)");
    println!("  • TCP_NODELAY enabled (disables Nagle's algorithm)");
    println!("  • HTTP/2 with keep-alive");
    println!("  • Optimized timeouts for different environments");
    println!("  • Compression enabled/disabled based on use case");
    
    println!("\nConfiguration Recommendations:");
    println!("  • Colocated: Use for servers close to exchange");
    println!("  • Internet: Use for retail/remote connections");
    println!("  • Standard: Balanced settings for most use cases");
    
    println!("\nAdditional Optimizations Available:");
    println!("  • Custom DNS resolver");
    println!("  • Connection pre-warming");
    println!("  • Request batching");
    println!("  • Circuit breaker patterns");
    
    Ok(())
}
