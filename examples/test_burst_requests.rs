use reqwest::Client;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("Testing Request Burst Patterns");
    println!("===============================\n");

    let client = Client::new();

    // Pattern 1: Burst (no delay) - simulates high-frequency trading
    println!("Pattern 1: Burst Requests (0ms delay)");
    println!("======================================");
    
    let mut burst_times = Vec::new();
    for i in 1..=10 {
        let start = Instant::now();
        let _ = client
            .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
            .send()
            .await?
            .bytes()
            .await?;
        let elapsed = start.elapsed();
        burst_times.push(elapsed);
        
        if i <= 5 {
            println!("  Request {}: {:.1} ms", i, elapsed.as_micros() as f64 / 1000.0);
        }
        // No delay - immediate next request
    }

    // Pattern 2: Short delay (50ms) - like our benchmark
    println!("\nPattern 2: Short Delay (50ms between requests)");
    println!("===============================================");
    
    let mut short_delay_times = Vec::new();
    for i in 1..=10 {
        let start = Instant::now();
        let _ = client
            .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
            .send()
            .await?
            .bytes()
            .await?;
        let elapsed = start.elapsed();
        short_delay_times.push(elapsed);
        
        if i <= 5 {
            println!("  Request {}: {:.1} ms", i, elapsed.as_micros() as f64 / 1000.0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Pattern 3: Medium delay (100ms) - our current benchmark
    println!("\nPattern 3: Medium Delay (100ms between requests)");
    println!("=================================================");
    
    let mut medium_delay_times = Vec::new();
    for i in 1..=10 {
        let start = Instant::now();
        let _ = client
            .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
            .send()
            .await?
            .bytes()
            .await?;
        let elapsed = start.elapsed();
        medium_delay_times.push(elapsed);
        
        if i <= 5 {
            println!("  Request {}: {:.1} ms", i, elapsed.as_micros() as f64 / 1000.0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Statistics
    fn calc_stats(times: &[std::time::Duration]) -> (f64, f64, f64, f64) {
        let values: Vec<f64> = times.iter().map(|d| d.as_micros() as f64 / 1000.0).collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        let std_dev = variance.sqrt();
        let mut sorted = values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (mean, std_dev, sorted[0], sorted[sorted.len() - 1])
    }

    let (burst_mean, burst_std, burst_min, burst_max) = calc_stats(&burst_times);
    let (short_mean, short_std, short_min, short_max) = calc_stats(&short_delay_times);
    let (med_mean, med_std, med_min, med_max) = calc_stats(&medium_delay_times);

    println!("\n\n📊 RESULTS");
    println!("==========\n");
    
    println!("Burst (0ms delay):");
    println!("  Mean: {:.1} ms ± {:.1} ms", burst_mean, burst_std);
    println!("  Range: {:.1} - {:.1} ms", burst_min, burst_max);
    println!("  First request: {:.1} ms", burst_times[0].as_micros() as f64 / 1000.0);
    println!("  Avg of requests 2-10: {:.1} ms", 
        burst_times.iter().skip(1).sum::<std::time::Duration>().as_millis() as f64 / 9.0);
    
    println!("\nShort Delay (50ms):");
    println!("  Mean: {:.1} ms ± {:.1} ms", short_mean, short_std);
    println!("  Range: {:.1} - {:.1} ms", short_min, short_max);
    
    println!("\nMedium Delay (100ms):");
    println!("  Mean: {:.1} ms ± {:.1} ms", med_mean, med_std);
    println!("  Range: {:.1} - {:.1} ms", med_min, med_max);

    println!("\n💡 INSIGHTS");
    println!("============\n");
    
    if burst_mean < short_mean && burst_mean < med_mean {
        let improvement_vs_100ms = ((med_mean - burst_mean) / med_mean) * 100.0;
        println!("✅ Burst requests are fastest: {:.1}% faster than 100ms delay", improvement_vs_100ms);
        println!("   This confirms connection reuse is critical!");
        
        let warm_avg = burst_times.iter().skip(1).sum::<std::time::Duration>().as_millis() as f64 / 9.0;
        let first = burst_times[0].as_micros() as f64 / 1000.0;
        println!("   First request (cold): {:.1} ms", first);
        println!("   Subsequent (warm): {:.1} ms", warm_avg);
        println!("   Connection reuse benefit: {:.1}%", ((first - warm_avg) / first) * 100.0);
    }
    
    if burst_std < med_std {
        println!("✅ Burst requests are more consistent: ±{:.1} ms vs ±{:.1} ms", burst_std, med_std);
    }
    
    println!("\n🎯 RECOMMENDATION");
    println!("==================");
    println!("For real-world high-frequency trading:");
    println!("  - Expected latency: {:.1} ms ± {:.1} ms (with warm connection)", burst_mean, burst_std);
    println!("  - First request will be slower: ~{:.1} ms (connection establishment)", 
        burst_times[0].as_micros() as f64 / 1000.0);
    println!("  - Keep client alive between requests for best performance");

    Ok(())
}
