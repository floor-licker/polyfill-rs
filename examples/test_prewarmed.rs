use polyfill_rs::ClobClient;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("Connection Pre-warming Test");
    println!("===========================\n");

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

    println!("Phase 1: Cold start (no pre-warming)");
    println!("=====================================");
    
    // Make 3 requests cold
    let mut cold_times = Vec::new();
    for i in 1..=3 {
        let start = Instant::now();
        let response = client.http_client
            .get(format!("{}/simplified-markets?next_cursor=MA==", client.base_url))
            .send()
            .await?;
        let _json: serde_json::Value = response.json().await?;
        let elapsed = start.elapsed();
        cold_times.push(elapsed);
        println!("Request {}: {:.1} ms", i, elapsed.as_micros() as f64 / 1000.0);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    println!("\nPhase 2: Pre-warmed connection");
    println!("================================");
    
    // Pre-warm by making several requests
    println!("Pre-warming with 5 requests...");
    for _ in 0..5 {
        let _ = client.http_client
            .get(format!("{}/simplified-markets?next_cursor=MA==", client.base_url))
            .send()
            .await?
            .bytes()
            .await?;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    
    println!("Testing with warmed connection...\n");
    
    // Now test with warmed connection
    let mut warm_times = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        let response = client.http_client
            .get(format!("{}/simplified-markets?next_cursor=MA==", client.base_url))
            .send()
            .await?;
        let _json: serde_json::Value = response.json().await?;
        let elapsed = start.elapsed();
        warm_times.push(elapsed);
        
        if i <= 5 {
            println!("Request {}: {:.1} ms", i, elapsed.as_micros() as f64 / 1000.0);
        }
        
        // Minimal delay
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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

    let (cold_mean, cold_std, cold_min, cold_max) = calc_stats(&cold_times);
    let (warm_mean, warm_std, warm_min, warm_max) = calc_stats(&warm_times);

    println!("\n\nResults:");
    println!("========\n");
    
    println!("Cold Start:");
    println!("  Mean: {:.1} ms ± {:.1} ms", cold_mean, cold_std);
    println!("  Range: {:.1} - {:.1} ms\n", cold_min, cold_max);
    
    println!("Pre-warmed:");
    println!("  Mean: {:.1} ms ± {:.1} ms", warm_mean, warm_std);
    println!("  Range: {:.1} - {:.1} ms", warm_min, warm_max);
    
    let improvement = ((cold_mean - warm_mean) / cold_mean) * 100.0;
    let variance_reduction = ((cold_std - warm_std) / cold_std) * 100.0;
    
    println!("\nImprovement:");
    println!("  Speed: {:.1}% faster", improvement);
    println!("  Variance: {:.1}% more consistent", variance_reduction);
    
    if warm_std < 30.0 {
        println!("\nSUCCESS: Achieved target variance (±{:.1}ms < ±30ms)", warm_std);
    } else {
        println!("\nStill need work: Current ±{:.1}ms, target ±30ms", warm_std);
        println!("Remaining variance is likely server-side or network conditions");
    }

    Ok(())
}
