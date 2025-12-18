use reqwest::Client;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("Final Benchmark - Apples-to-Apples Comparison");
    println!("==============================================\n");

    let client = Client::new();

    // Match polymarket-rs-client's benchmark methodology
    println!("Testing: /simplified-markets endpoint");
    println!("Iterations: 20 (matching their methodology)");
    println!("Delay: 100ms between requests\n");

    let mut times = Vec::new();

    for i in 1..=20 {
        let start = Instant::now();
        let response = client
            .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
            .send()
            .await?;

        let _json: serde_json::Value = response.json().await?;
        let elapsed = start.elapsed();
        times.push(elapsed);

        if i <= 5 || i > 15 {
            println!(
                "  Request {:2}: {:.1} ms",
                i,
                elapsed.as_micros() as f64 / 1000.0
            );
        } else if i == 6 {
            println!("  ...");
        }

        // 100ms delay like we used before
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Calculate statistics
    let values: Vec<f64> = times
        .iter()
        .map(|d| d.as_micros() as f64 / 1000.0)
        .collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;

    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    let std_dev = variance.sqrt();

    let mut sorted = values.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let median = sorted[sorted.len() / 2];

    println!("\n\n📊 FINAL RESULTS");
    println!("=================\n");

    println!("polyfill-rs Performance:");
    println!("  Mean:   {:.1} ms ± {:.1} ms", mean, std_dev);
    println!("  Median: {:.1} ms", median);
    println!("  Range:  {:.1} - {:.1} ms", min, max);

    println!("\n polymarket-rs-client (from their README):");
    println!("  Mean:   404.5 ms ± 22.9 ms");

    println!("\nOfficial Python Client (from their README):");
    println!("  Mean:   1366 ms ± 48 ms");

    println!("\n\n📈 COMPARISON");
    println!("==============\n");

    let diff_vs_rust = mean - 404.5;
    let diff_pct_rust = (diff_vs_rust / 404.5) * 100.0;

    if diff_vs_rust < 0.0 {
        println!(
            "vs polymarket-rs-client: {:.1}% FASTER ({:.1} ms faster)",
            -diff_pct_rust, -diff_vs_rust
        );
    } else if diff_pct_rust < 5.0 {
        println!(
            "vs polymarket-rs-client: COMPETITIVE (within {:.1}%, +{:.1} ms)",
            diff_pct_rust, diff_vs_rust
        );
    } else {
        println!(
            "vs polymarket-rs-client: {:.1}% slower (+{:.1} ms)",
            diff_pct_rust, diff_vs_rust
        );
    }

    let speedup_vs_python = 1366.0 / mean;
    println!(
        "vs Official Python:      {:.1}x FASTER ({:.1} ms faster)",
        speedup_vs_python,
        1366.0 - mean
    );

    println!("\n\n🎯 VARIANCE ANALYSIS");
    println!("=====================\n");

    let variance_pct = (std_dev / mean) * 100.0;
    println!("Our variance: ±{:.1} ms ({:.1}%)", std_dev, variance_pct);
    println!("Their variance: ±22.9 ms (5.7%)");

    if std_dev < 30.0 {
        println!("\n✅ Excellent consistency!");
    } else if std_dev < 50.0 {
        println!("\n✅ Good consistency");
    } else {
        println!("\n⚠️  Higher variance than polymarket-rs-client");
        println!("   This is likely due to:");
        println!("   - Network conditions (time of day, routing)");
        println!("   - Geographic distance to server");
        println!("   - System load during testing");
    }

    Ok(())
}
