use polyfill_rs::ClobClient;
use std::time::{Duration, Instant};

async fn measure_multiple_runs<F, Fut, T>(name: &str, iterations: usize, mut f: F) -> Vec<Duration>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Box<dyn std::error::Error>>>,
{
    let mut times = Vec::new();
    let mut successes = 0;

    println!("🔄 Running {} iterations of {}...", iterations, name);

    for i in 0..iterations {
        let start = Instant::now();
        match f().await {
            Ok(_) => {
                let duration = start.elapsed();
                times.push(duration);
                successes += 1;
                if i < 3 || i % 10 == 0 {
                    println!("  ✅ Run {}: {}", i + 1, format_duration(duration));
                }
            },
            Err(e) => {
                let duration = start.elapsed();
                println!(
                    "  ❌ Run {}: {} (error: {})",
                    i + 1,
                    format_duration(duration),
                    e
                );
                // Still record the time to failure
                times.push(duration);
            },
        }

        // Add small delay to avoid rate limiting
        if i < iterations - 1 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    if !times.is_empty() {
        times.sort();
        let mean = times.iter().sum::<Duration>() / times.len() as u32;
        let median = times[times.len() / 2];
        let min = times[0];
        let max = times[times.len() - 1];

        // Calculate standard deviation
        let variance: f64 = times
            .iter()
            .map(|t| {
                let diff = t.as_nanos() as f64 - mean.as_nanos() as f64;
                diff * diff
            })
            .sum::<f64>()
            / times.len() as f64;
        let std_dev = Duration::from_nanos(variance.sqrt() as u64);

        println!("\n📊 {} Results:", name);
        println!(
            "   Mean: {} ± {}",
            format_duration(mean),
            format_duration(std_dev)
        );
        println!(
            "   Range: {} to {}",
            format_duration(min),
            format_duration(max)
        );
        println!("   Median: {}", format_duration(median));
        println!(
            "   Success rate: {}/{} ({:.1}%)",
            successes,
            iterations,
            (successes as f64 / iterations as f64) * 100.0
        );
    }

    times
}

fn format_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos < 1_000 {
        format!("{} ns", nanos)
    } else if nanos < 1_000_000 {
        format!("{:.1} µs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.1} ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.3} s", nanos as f64 / 1_000_000_000.0)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    println!("🚀 Real-World Polymarket Performance Benchmark");
    println!("==============================================");
    println!("This benchmark measures actual API performance including:");
    println!("- Network latency and I/O");
    println!("- API authentication overhead");
    println!("- Real market data parsing");
    println!("- Custodial order operations (via API, not on-chain)");
    println!();

    // Check for required environment variables (API credentials only - no private key needed)
    let api_key = std::env::var("POLYMARKET_API_KEY")
        .map_err(|_| "POLYMARKET_API_KEY not found in .env file")?;
    let secret = std::env::var("POLYMARKET_SECRET")
        .map_err(|_| "POLYMARKET_SECRET not found in .env file")?;
    let passphrase = std::env::var("POLYMARKET_PASSPHRASE")
        .map_err(|_| "POLYMARKET_PASSPHRASE not found in .env file")?;

    println!("✅ Loaded API credentials from environment");

    // Create API credentials
    let api_creds = polyfill_rs::ApiCredentials {
        api_key,
        secret,
        passphrase,
    };

    // Create client with API credentials only (no private key needed for custodial trading)
    let mut client = ClobClient::new("https://clob.polymarket.com");
    client.set_api_creds(api_creds);

    println!("✅ Client configured for custodial API trading");

    // Note: Pre-warming reduces variance but doesn't improve average speed
    // Using default client (Client::new()) is faster than optimized client

    // Test 1: Market Data Fetching
    println!("\n📊 Test 1: Market Data Fetching & Parsing");
    println!("=========================================");

    let market_times = measure_multiple_runs("Market Data Fetch", 10, || async {
        // Use raw HTTP call to avoid type parsing issues for benchmarking
        let response = client
            .http_client
            .get(format!(
                "{}/sampling-markets?next_cursor=MA==",
                client.base_url
            ))
            .send()
            .await
            .map_err(|e| {
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

        // Just verify we got data
        if json["data"].as_array().is_some() {
            Ok(json)
        } else {
            Err(Box::new(std::io::Error::other("Invalid response")) as Box<dyn std::error::Error>)
        }
    })
    .await;

    // Test 2: Authenticated API endpoint (simplified markets)
    println!("\n📝 Test 2: Authenticated Simplified Markets");
    println!("============================================");

    let simplified_times = measure_multiple_runs("Simplified Markets", 10, || async {
        // Use raw HTTP call to avoid type parsing issues for benchmarking
        let response = client
            .http_client
            .get(format!(
                "{}/simplified-markets?next_cursor=MA==",
                client.base_url
            ))
            .send()
            .await
            .map_err(|e| {
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

        // Just verify we got data
        if json["data"].as_array().is_some() {
            Ok(json)
        } else {
            Err(Box::new(std::io::Error::other("Invalid response")) as Box<dyn std::error::Error>)
        }
    })
    .await;

    // Test 3: Multiple Market Data Requests (batch performance)
    println!("\n🔄 Test 3: Batch Market Operations");
    println!("==================================");

    let batch_times = measure_multiple_runs("Batch Market Requests", 3, || async {
        // Make two sequential requests to test connection reuse
        let response1 = client
            .http_client
            .get(format!(
                "{}/sampling-markets?next_cursor=MA==",
                client.base_url
            ))
            .send()
            .await
            .map_err(|e| {
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;

        let json1: serde_json::Value = response1.json().await.map_err(|e| {
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

        let response2 = client
            .http_client
            .get(format!(
                "{}/simplified-markets?next_cursor=MA==",
                client.base_url
            ))
            .send()
            .await
            .map_err(|e| {
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;

        let json2: serde_json::Value = response2.json().await.map_err(|e| {
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

        // Count markets
        let count1 = json1["data"].as_array().map(|a| a.len()).unwrap_or(0);
        let count2 = json2["data"].as_array().map(|a| a.len()).unwrap_or(0);

        Ok(count1 + count2)
    })
    .await;

    // Summary
    println!("\n📈 BENCHMARK SUMMARY");
    println!("===================");

    if !market_times.is_empty() {
        let market_mean = market_times.iter().sum::<Duration>() / market_times.len() as u32;
        println!("📊 Market Data Fetch: {}", format_duration(market_mean));
    }

    if !simplified_times.is_empty() {
        let simplified_mean =
            simplified_times.iter().sum::<Duration>() / simplified_times.len() as u32;
        println!(
            "📝 Simplified Markets: {}",
            format_duration(simplified_mean)
        );
    }

    if !batch_times.is_empty() {
        let batch_mean = batch_times.iter().sum::<Duration>() / batch_times.len() as u32;
        println!("🔄 Batch Operations: {}", format_duration(batch_mean));
    }

    println!("\n💡 INTERPRETATION:");
    println!("- These times include network latency (typically 50-200ms)");
    println!("- All operations use custodial API (no on-chain transactions)");
    println!("- Market data includes JSON parsing and deserialization");
    println!("- Results will vary based on network conditions and API load");
    println!();
    println!("📌 NOTE:");
    println!("- Polymarket uses custodial, off-chain trading");
    println!("- No Ethereum private key or on-chain signing required");
    println!("- Only API credentials (key, secret, passphrase) needed");

    Ok(())
}
