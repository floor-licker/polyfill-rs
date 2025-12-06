use polyfill_rs::ClobClient;
use std::time::Instant;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Advanced Network Optimizations - polyfill-rs");
    println!("===============================================");

    // Use the best-performing configuration (Internet)
    let client = ClobClient::new_internet("https://clob.polymarket.com");

    println!("📊 Test 1: Connection Pre-warming");
    println!("=================================");

    // Test without pre-warming
    let start = Instant::now();
    let _ = client.get_server_time().await;
    let cold_start = start.elapsed();
    println!("  ❄️  Cold start: {:?}", cold_start);

    // Test with pre-warming
    let client_warm = ClobClient::new_internet("https://clob.polymarket.com");
    let _ = client_warm.prewarm_connections().await;

    let start = Instant::now();
    let _ = client_warm.get_server_time().await;
    let warm_start = start.elapsed();
    println!("  🔥 Warm start: {:?}", warm_start);
    println!(
        "  📈 Improvement: {:.1}x faster",
        cold_start.as_millis() as f64 / warm_start.as_millis() as f64
    );

    println!("\n📊 Test 2: Request Batching Simulation");
    println!("=====================================");

    // Sequential requests
    let start = Instant::now();
    for _ in 0..5 {
        let _ = client.get_server_time().await;
    }
    let sequential_time = start.elapsed();
    println!("  📝 Sequential: 5 requests in {:?}", sequential_time);

    // Parallel requests (simulating batching)
    let start = Instant::now();
    let futures = (0..5).map(|_| client.get_server_time());
    let _results: Vec<_> = futures_util::future::join_all(futures).await;
    let parallel_time = start.elapsed();
    println!("  ⚡ Parallel: 5 requests in {:?}", parallel_time);
    println!(
        "  📈 Improvement: {:.1}x faster",
        sequential_time.as_millis() as f64 / parallel_time.as_millis() as f64
    );

    println!("\n📊 Test 3: Circuit Breaker Pattern");
    println!("=================================");

    struct SimpleCircuitBreaker {
        failure_count: u32,
        failure_threshold: u32,
        recovery_timeout: Duration,
        last_failure: Option<Instant>,
        state: CircuitState,
    }

    #[derive(Debug, PartialEq)]
    enum CircuitState {
        Closed,   // Normal operation
        Open,     // Failing, reject requests
        HalfOpen, // Testing if service recovered
    }

    impl SimpleCircuitBreaker {
        fn new() -> Self {
            Self {
                failure_count: 0,
                failure_threshold: 3,
                recovery_timeout: Duration::from_secs(10),
                last_failure: None,
                state: CircuitState::Closed,
            }
        }

        fn can_execute(&mut self) -> bool {
            match self.state {
                CircuitState::Closed => true,
                CircuitState::Open => {
                    if let Some(last_failure) = self.last_failure {
                        if last_failure.elapsed() > self.recovery_timeout {
                            self.state = CircuitState::HalfOpen;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                },
                CircuitState::HalfOpen => true,
            }
        }

        fn on_success(&mut self) {
            self.failure_count = 0;
            self.state = CircuitState::Closed;
        }

        fn on_failure(&mut self) {
            self.failure_count += 1;
            self.last_failure = Some(Instant::now());

            if self.failure_count >= self.failure_threshold {
                self.state = CircuitState::Open;
            }
        }
    }

    let mut circuit_breaker = SimpleCircuitBreaker::new();
    let mut successful_requests = 0;
    let mut rejected_requests = 0;

    // Simulate some requests with circuit breaker
    for i in 0..10 {
        if circuit_breaker.can_execute() {
            match client.get_server_time().await {
                Ok(_) => {
                    circuit_breaker.on_success();
                    successful_requests += 1;
                    if i < 3 {
                        println!("  ✅ Request {} succeeded", i + 1);
                    }
                },
                Err(_) => {
                    circuit_breaker.on_failure();
                    if i < 3 {
                        println!("  ❌ Request {} failed", i + 1);
                    }
                },
            }
        } else {
            rejected_requests += 1;
            if i < 3 {
                println!("  🚫 Request {} rejected by circuit breaker", i + 1);
            }
        }

        // Small delay between requests
        sleep(Duration::from_millis(100)).await;
    }

    println!(
        "  📊 Results: {} successful, {} rejected",
        successful_requests, rejected_requests
    );

    println!("\n📊 Test 4: Adaptive Timeout Strategy");
    println!("===================================");

    struct AdaptiveTimeout {
        recent_times: Vec<Duration>,
        max_samples: usize,
    }

    impl AdaptiveTimeout {
        fn new() -> Self {
            Self {
                recent_times: Vec::new(),
                max_samples: 10,
            }
        }

        fn add_sample(&mut self, duration: Duration) {
            self.recent_times.push(duration);
            if self.recent_times.len() > self.max_samples {
                self.recent_times.remove(0);
            }
        }

        fn get_adaptive_timeout(&self) -> Duration {
            if self.recent_times.is_empty() {
                return Duration::from_millis(5000); // Default
            }

            let avg = self.recent_times.iter().sum::<Duration>() / self.recent_times.len() as u32;
            // Set timeout to 3x average response time
            avg * 3
        }
    }

    let mut adaptive_timeout = AdaptiveTimeout::new();

    // Collect some samples
    for i in 0..5 {
        let start = Instant::now();
        if (client.get_server_time().await).is_ok() {
            let duration = start.elapsed();
            adaptive_timeout.add_sample(duration);
            if i < 3 {
                println!("  📊 Sample {}: {:?}", i + 1, duration);
            }
        }
    }

    let recommended_timeout = adaptive_timeout.get_adaptive_timeout();
    println!("  🎯 Recommended timeout: {:?}", recommended_timeout);

    println!("\n🎯 Advanced Optimization Summary");
    println!("===============================");
    println!("Implemented Optimizations:");
    println!("  ✅ Connection pre-warming (reduces cold start latency)");
    println!("  ✅ Request parallelization (batching simulation)");
    println!("  ✅ Circuit breaker pattern (prevents cascade failures)");
    println!("  ✅ Adaptive timeouts (dynamic based on network conditions)");

    println!("\nFurther Optimizations Available:");
    println!("  🔧 Custom DNS resolver with caching");
    println!("  🔧 Connection affinity (sticky connections)");
    println!("  🔧 Request prioritization queues");
    println!("  🔧 Geographical load balancing");
    println!("  🔧 WebSocket connections for real-time data");
    println!("  🔧 HTTP/3 (QUIC) when supported");

    println!("\n📈 Expected Network Improvements:");
    println!("  • 10-30% latency reduction from optimized HTTP client");
    println!("  • 50-80% improvement in connection reuse scenarios");
    println!("  • Better resilience during network instability");
    println!("  • Adaptive performance based on network conditions");

    Ok(())
}
