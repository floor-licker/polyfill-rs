use polyfill_rs::ClobClient;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🌐 Network Latency Test for polyfill-rs");
    println!("======================================");
    
    let client = ClobClient::new("https://clob.polymarket.com");
    
    // Test 1: Simplified markets (comparable to original 404.5ms benchmark)
    println!("\n📊 Test 1: Simplified Markets");
    println!("-----------------------------");
    
    let mut times = Vec::new();
    for i in 0..5 {
        let start = Instant::now();
        match client.get_sampling_simplified_markets(None).await {
            Ok(markets) => {
                let duration = start.elapsed();
                times.push(duration);
                println!("  Run {}: ✅ {} markets in {:?}", i+1, markets.data.len(), duration);
            }
            Err(e) => {
                let duration = start.elapsed();
                times.push(duration);
                println!("  Run {}: ❌ Error in {:?}: {}", i+1, duration, e);
            }
        }
    }
    
    if !times.is_empty() {
        let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        let min = times.iter().min().unwrap();
        let max = times.iter().max().unwrap();
        
        println!("  📈 Average: {:?}", avg);
        println!("  📊 Range: {:?} - {:?}", min, max);
        println!("  🆚 vs original (404.5ms): {:.1}x", 
                 404.5 / avg.as_millis() as f64);
    }
    
    // Test 2: Full markets
    println!("\n📊 Test 2: Full Markets");
    println!("----------------------");
    
    let mut times = Vec::new();
    for i in 0..3 {
        let start = Instant::now();
        match client.get_sampling_markets(None).await {
            Ok(markets) => {
                let duration = start.elapsed();
                times.push(duration);
                println!("  Run {}: ✅ {} markets in {:?}", i+1, markets.data.len(), duration);
            }
            Err(e) => {
                let duration = start.elapsed();
                times.push(duration);
                println!("  Run {}: ❌ Error in {:?}: {}", i+1, duration, e);
            }
        }
    }
    
    if !times.is_empty() {
        let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        let min = times.iter().min().unwrap();
        let max = times.iter().max().unwrap();
        
        println!("  📈 Average: {:?}", avg);
        println!("  📊 Range: {:?} - {:?}", min, max);
    }
    
    // Test 3: Server time (lightweight endpoint)
    println!("\n📊 Test 3: Server Time (Lightweight)");
    println!("-----------------------------------");
    
    let mut times = Vec::new();
    for i in 0..10 {
        let start = Instant::now();
        match client.get_server_time().await {
            Ok(timestamp) => {
                let duration = start.elapsed();
                times.push(duration);
                if i == 0 {
                    println!("  Run {}: ✅ Timestamp {} in {:?}", i+1, timestamp, duration);
                }
            }
            Err(e) => {
                let duration = start.elapsed();
                times.push(duration);
                println!("  Run {}: ❌ Error in {:?}: {}", i+1, duration, e);
            }
        }
    }
    
    if !times.is_empty() {
        let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        let min = times.iter().min().unwrap();
        let max = times.iter().max().unwrap();
        
        println!("  📈 Average: {:?}", avg);
        println!("  📊 Range: {:?} - {:?}", min, max);
        println!("  🌐 Network baseline latency: ~{:?}", min);
    }
    
    println!("\n🎯 Summary");
    println!("=========");
    println!("Network latency dominates end-to-end performance.");
    println!("Our computational optimizations provide benefits when:");
    println!("• Processing cached/local data");
    println!("• Running in co-located environments");
    println!("• Performing high-frequency operations");
    println!();
    println!("For fair comparison with polymarket-rs-client:");
    println!("• Run from same geographic location");
    println!("• Use same network conditions");
    println!("• Measure full end-to-end latency");
    
    Ok(())
}
