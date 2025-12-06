use polyfill_rs::decode::fast_parse;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("SIMD JSON Parsing Benchmark");
    println!("============================\n");

    // Fetch real data to parse
    let client = reqwest::Client::new();
    let response = client
        .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
        .send()
        .await?;
    
    let data = response.bytes().await?;
    println!("Response size: {} KB\n", data.len() / 1024);

    // Test 1: Standard serde_json
    println!("Test 1: Standard serde_json");
    println!("----------------------------");
    let mut serde_times = Vec::new();
    
    for i in 1..=10 {
        let data_copy = data.clone();
        let start = Instant::now();
        let _json: serde_json::Value = serde_json::from_slice(&data_copy)?;
        let elapsed = start.elapsed();
        serde_times.push(elapsed);
        
        if i <= 3 {
            println!("  Run {}: {:.2} ms", i, elapsed.as_micros() as f64 / 1000.0);
        }
    }

    // Test 2: SIMD JSON
    println!("\nTest 2: SIMD JSON (simd-json)");
    println!("------------------------------");
    let mut simd_times = Vec::new();
    
    for i in 1..=10 {
        let mut data_copy = data.to_vec();
        let start = Instant::now();
        let _json: serde_json::Value = fast_parse::parse_json_fast(&mut data_copy)?;
        let elapsed = start.elapsed();
        simd_times.push(elapsed);
        
        if i <= 3 {
            println!("  Run {}: {:.2} ms", i, elapsed.as_micros() as f64 / 1000.0);
        }
    }

    // Statistics
    let serde_avg = serde_times.iter().sum::<std::time::Duration>().as_micros() as f64 / serde_times.len() as f64 / 1000.0;
    let simd_avg = simd_times.iter().sum::<std::time::Duration>().as_micros() as f64 / simd_times.len() as f64 / 1000.0;

    println!("\n\nResults:");
    println!("========");
    println!("serde_json:  {:.2} ms", serde_avg);
    println!("simd-json:   {:.2} ms", simd_avg);
    
    let speedup = serde_avg / simd_avg;
    let improvement = ((serde_avg - simd_avg) / serde_avg) * 100.0;
    
    println!("\nSpeedup: {:.2}x ({:.1}% faster)", speedup, improvement);
    println!("Time saved per request: {:.2} ms", serde_avg - simd_avg);

    Ok(())
}
