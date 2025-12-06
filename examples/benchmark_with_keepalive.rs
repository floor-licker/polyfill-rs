use polyfill_rs::ClobClient;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Benchmark with Keep-Alive Enabled");
    println!("==================================\n");

    let mut client = ClobClient::new("https://clob.polymarket.com");
    
    // Start keep-alive
    println!("Starting keep-alive...");
    client.start_keepalive(Duration::from_secs(30)).await;
    
    // Give it a moment to establish
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("Keep-alive started\n");

    println!("Testing: /simplified-markets endpoint");
    println!("Iterations: 20");
    println!("Delay: 100ms between requests\n");

    let mut times = Vec::new();
    
    for i in 1..=20 {
        let start = Instant::now();
        let response = client.http_client
            .get(format!("{}/simplified-markets?next_cursor=MA==", client.base_url))
            .send()
            .await?;
        
        let _json: serde_json::Value = response.json().await?;
        let elapsed = start.elapsed();
        times.push(elapsed);
        
        if i <= 5 || i > 15 {
            println!("  Request {:2}: {:.1} ms", i, elapsed.as_micros() as f64 / 1000.0);
        } else if i == 6 {
            println!("  ...");
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Stop keep-alive
    client.stop_keepalive().await;

    // Calculate statistics
    let values: Vec<f64> = times.iter().map(|d| d.as_micros() as f64 / 1000.0).collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    
    let variance = values.iter()
        .map(|v| (v - mean).powi(2))
        .sum::<f64>() / values.len() as f64;
    let std_dev = variance.sqrt();
    
    let mut sorted = values.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let median = sorted[sorted.len() / 2];

    println!("\n\n📊 RESULTS WITH KEEP-ALIVE");
    println!("===========================\n");
    
    println!("Mean:   {:.1} ms ± {:.1} ms", mean, std_dev);
    println!("Median: {:.1} ms", median);
    println!("Range:  {:.1} - {:.1} ms", min, max);
    
    println!("\nvs polymarket-rs-client: 404.5 ms ± 22.9 ms");
    println!("vs previous (no keep-alive): 382.6 ms ± 75.1 ms");
    
    let diff = mean - 404.5;
    if diff < 0.0 {
        println!("\n✅ {:.1}% FASTER than polymarket-rs-client", -diff / 404.5 * 100.0);
    } else {
        println!("\n⚠️ {:.1}% slower than polymarket-rs-client", diff / 404.5 * 100.0);
    }

    Ok(())
}
