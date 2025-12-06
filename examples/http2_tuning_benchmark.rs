use reqwest::ClientBuilder;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("HTTP/2 Configuration Tuning Benchmark");
    println!("======================================\n");
    println!("Testing various HTTP/2 settings to find optimal configuration");
    println!("Each configuration runs 20 iterations\n");

    // Test matrix
    let stream_windows = vec![
        512 * 1024,      // 512KB
        1024 * 1024,     // 1MB
        2 * 1024 * 1024, // 2MB
        4 * 1024 * 1024, // 4MB
        8 * 1024 * 1024, // 8MB
    ];

    let connection_windows = vec![
        1024 * 1024,      // 1MB
        2 * 1024 * 1024,  // 2MB
        4 * 1024 * 1024,  // 4MB
        8 * 1024 * 1024,  // 8MB
        16 * 1024 * 1024, // 16MB
    ];

    let max_frame_sizes = vec![
        None,             // Default (16KB)
        Some(32 * 1024),  // 32KB
        Some(64 * 1024),  // 64KB
    ];

    let keep_alive_intervals = vec![
        Duration::from_secs(10),
        Duration::from_secs(20),
        Duration::from_secs(30),
        Duration::from_secs(60),
    ];

    let mut best_config = None;
    let mut best_mean = f64::MAX;

    // Test 1: Baseline (default client)
    println!("Baseline: Default Client");
    println!("-------------------------");
    let baseline_mean = test_config(reqwest::Client::new(), "Default").await?;
    best_mean = baseline_mean;
    best_config = Some("Default Client".to_string());

    // Test 2: Stream window sizes (with default connection window)
    println!("\n\nTest 2: Stream Window Sizes");
    println!("============================");
    for stream_window in &stream_windows {
        let client = ClientBuilder::new()
            .http2_adaptive_window(true)
            .http2_initial_stream_window_size(*stream_window as u32)
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;

        let name = format!("Stream: {}KB", stream_window / 1024);
        let mean = test_config(client, &name).await?;

        if mean < best_mean {
            best_mean = mean;
            best_config = Some(name.clone());
        }
    }

    // Test 3: Connection window sizes (with best stream window from above)
    println!("\n\nTest 3: Connection Window Sizes");
    println!("================================");
    
    // Use 2MB stream window as a reasonable default for this test
    let default_stream_window = 2 * 1024 * 1024;
    
    for conn_window in &connection_windows {
        let client = ClientBuilder::new()
            .http2_adaptive_window(true)
            .http2_initial_stream_window_size(default_stream_window)
            .http2_initial_connection_window_size(*conn_window as u32)
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;

        let name = format!("Conn: {}MB", conn_window / (1024 * 1024));
        let mean = test_config(client, &name).await?;

        if mean < best_mean {
            best_mean = mean;
            best_config = Some(name.clone());
        }
    }

    // Test 4: Max frame sizes
    println!("\n\nTest 4: Max Frame Sizes");
    println!("========================");
    for frame_size in &max_frame_sizes {
        let mut builder = ClientBuilder::new()
            .http2_adaptive_window(true)
            .http2_initial_stream_window_size(default_stream_window)
            .http2_initial_connection_window_size(4 * 1024 * 1024)
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90));

        if let Some(size) = frame_size {
            builder = builder.http2_max_frame_size(Some(*size));
        }

        let client = builder.build()?;

        let name = match frame_size {
            None => "Frame: Default".to_string(),
            Some(s) => format!("Frame: {}KB", s / 1024),
        };
        
        let mean = test_config(client, &name).await?;

        if mean < best_mean {
            best_mean = mean;
            best_config = Some(name.clone());
        }
    }

    // Test 5: Keep-alive intervals
    println!("\n\nTest 5: Keep-Alive Intervals");
    println!("=============================");
    for interval in &keep_alive_intervals {
        let client = ClientBuilder::new()
            .http2_adaptive_window(true)
            .http2_initial_stream_window_size(default_stream_window)
            .http2_initial_connection_window_size(4 * 1024 * 1024)
            .http2_keep_alive_interval(*interval)
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;

        let name = format!("Keep-alive: {}s", interval.as_secs());
        let mean = test_config(client, &name).await?;

        if mean < best_mean {
            best_mean = mean;
            best_config = Some(name.clone());
        }
    }

    // Summary
    println!("\n\n");
    println!("═══════════════════════════════════════");
    println!("            FINAL RESULTS              ");
    println!("═══════════════════════════════════════");
    println!("\nBest Configuration: {}", best_config.unwrap());
    println!("Best Mean Latency: {:.1} ms", best_mean);
    println!("\nBaseline (default): {:.1} ms", baseline_mean);
    
    let improvement = ((baseline_mean - best_mean) / baseline_mean) * 100.0;
    if improvement > 0.0 {
        println!("Improvement: {:.1}% faster", improvement);
    } else {
        println!("Note: Default client is fastest!");
    }

    Ok(())
}

async fn test_config(client: reqwest::Client, name: &str) -> Result<f64, Box<dyn std::error::Error>> {
    let iterations = 20;
    let mut times = Vec::new();

    print!("  Testing {}... ", name);
    
    for _ in 0..iterations {
        let start = Instant::now();
        
        match client
            .get("https://clob.polymarket.com/simplified-markets?next_cursor=MA==")
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    let _ = response.bytes().await;
                    times.push(start.elapsed());
                }
            }
            Err(_) => {
                // Skip failed requests
                continue;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    if times.is_empty() {
        println!("FAILED (all requests failed)");
        return Ok(f64::MAX);
    }

    let mean = times.iter().sum::<Duration>().as_millis() as f64 / times.len() as f64;
    let variance = times.iter()
        .map(|t| {
            let diff = t.as_millis() as f64 - mean;
            diff * diff
        })
        .sum::<f64>() / times.len() as f64;
    let std_dev = variance.sqrt();

    println!("{:.1} ms ± {:.1} ms", mean, std_dev);

    Ok(mean)
}

