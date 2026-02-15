//! Side-by-side benchmark comparing polyfill-rs vs a plain reqwest baseline.
//!
//! Run with:
//! `cargo run --example side_by_side_benchmark --features side-by-side-benchmark`

use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("═══════════════════════════════════════════════════════");
    println!("          SIDE-BY-SIDE BENCHMARK");
    println!("═══════════════════════════════════════════════════════");
    println!("\nTesting both clients on:");
    println!("  - Same machine");
    println!("  - Same network");
    println!("  - Same time");
    println!("  - Same API endpoint (/simplified-markets)");
    println!("  - 20 iterations each");
    println!("  - 100ms delay between requests\n");

    // Test 1: baseline reqwest
    println!("══════════════════════════════════════");
    println!("Test 1: baseline reqwest");
    println!("══════════════════════════════════════");

    let baseline_http = reqwest::Client::new();

    let mut their_times = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        match baseline_http
            .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
            .send()
            .await
        {
            Ok(response) => match response.json::<serde_json::Value>().await {
                Ok(_json) => {
                    let elapsed = start.elapsed();
                    their_times.push(elapsed);

                    if i <= 3 || i > 17 {
                        println!(
                            "  Request {:2}: {:.1} ms",
                            i,
                            elapsed.as_micros() as f64 / 1000.0
                        );
                    } else if i == 4 {
                        println!("  ...");
                    }
                },
                Err(e) => {
                    println!("  Request {:2}: PARSE ERROR - {}", i, e);
                },
            },
            Err(e) => {
                println!("  Request {:2}: NETWORK ERROR - {}", i, e);
            },
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Small break between tests
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Test 2: polyfill-rs (with keep-alive)
    println!("\n══════════════════════════════════════");
    println!("Test 2: polyfill-rs (with keep-alive)");
    println!("══════════════════════════════════════");

    let our_client = polyfill_rs::ClobClient::new("https://clob.polymarket.com");
    our_client
        .start_keepalive(std::time::Duration::from_secs(30))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await; // Let keep-alive establish

    let mut our_times = Vec::new();
    for i in 1..=20 {
        let start = Instant::now();
        match our_client
            .http_client
            .get(format!(
                "{}/simplified-markets?next_cursor=MA==",
                our_client.base_url
            ))
            .send()
            .await
        {
            Ok(response) => match response.json::<serde_json::Value>().await {
                Ok(_json) => {
                    let elapsed = start.elapsed();
                    our_times.push(elapsed);

                    if i <= 3 || i > 17 {
                        println!(
                            "  Request {:2}: {:.1} ms",
                            i,
                            elapsed.as_micros() as f64 / 1000.0
                        );
                    } else if i == 4 {
                        println!("  ...");
                    }
                },
                Err(e) => {
                    println!("  Request {:2}: PARSE ERROR - {}", i, e);
                },
            },
            Err(e) => {
                println!("  Request {:2}: NETWORK ERROR - {}", i, e);
            },
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    our_client.stop_keepalive().await;

    // Calculate statistics
    fn calc_stats(times: &[std::time::Duration]) -> (f64, f64, f64, f64, f64) {
        if times.is_empty() {
            return (0.0, 0.0, 0.0, 0.0, 0.0);
        }

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
        (mean, std_dev, min, max, median)
    }

    let (their_mean, their_std, their_min, their_max, their_median) = calc_stats(&their_times);
    let (our_mean, our_std, our_min, our_max, our_median) = calc_stats(&our_times);

    // Results
    println!("\n\n");
    println!("═══════════════════════════════════════════════════════");
    println!("                   HONEST RESULTS                      ");
    println!("═══════════════════════════════════════════════════════\n");

    println!("baseline reqwest:");
    println!("  Mean:     {:.1} ms ± {:.1} ms", their_mean, their_std);
    println!("  Median:   {:.1} ms", their_median);
    println!("  Range:    {:.1} - {:.1} ms", their_min, their_max);
    println!("  Variance: {:.1}%", (their_std / their_mean) * 100.0);
    println!("  Success:  {}/20 requests", their_times.len());

    println!("\n");
    println!("polyfill-rs (with keep-alive):");
    println!("  Mean:     {:.1} ms ± {:.1} ms", our_mean, our_std);
    println!("  Median:   {:.1} ms", our_median);
    println!("  Range:    {:.1} - {:.1} ms", our_min, our_max);
    println!("  Variance: {:.1}%", (our_std / our_mean) * 100.0);
    println!("  Success:  {}/20 requests", our_times.len());
    println!("\n  (We claimed in README: 368.6 ms ± 67.1 ms)");

    println!("\n");
    println!("═══════════════════════════════════════════════════════");

    if our_times.is_empty() || their_times.is_empty() {
        println!("ERROR: Not enough successful requests to compare");
    } else {
        let diff = our_mean - their_mean;
        let pct = (diff.abs() / their_mean) * 100.0;

        if diff < 0.0 {
            println!(
                "✅ polyfill-rs is {:.1}% FASTER ({:.1} ms faster)",
                pct, -diff
            );
        } else {
            println!(
                "❌ baseline reqwest is {:.1}% faster ({:.1} ms faster)",
                pct, diff
            );
        }
    }

    println!("═══════════════════════════════════════════════════════");

    // Detailed variance comparison
    println!("\n\nVariance Analysis:");
    println!("────────────────────────────────────────────────────");
    println!(
        "  baseline reqwest: ±{:.1} ms ({:.1}% variance)",
        their_std,
        (their_std / their_mean) * 100.0
    );
    println!(
        "  polyfill-rs:          ±{:.1} ms ({:.1}% variance)",
        our_std,
        (our_std / our_mean) * 100.0
    );
    println!();

    if our_std < their_std {
        let improvement = ((their_std - our_std) / their_std) * 100.0;
        println!("  ✅ polyfill-rs is {:.1}% more consistent", improvement);
    } else {
        let diff = ((our_std - their_std) / their_std) * 100.0;
        println!("  ⚠️  baseline reqwest is {:.1}% more consistent", diff);
    }

    Ok(())
}
