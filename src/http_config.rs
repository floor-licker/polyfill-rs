//! HTTP client optimization for low-latency trading
//!
//! This module provides optimized HTTP client configurations specifically
//! designed for high-frequency trading environments where every millisecond counts.

use reqwest::{Client, ClientBuilder};
use std::time::Duration;

/// Connection pre-warming helper
pub async fn prewarm_connections(client: &Client, base_url: &str) -> Result<(), reqwest::Error> {
    // GET request to establish TCP/TLS connection
    let _ = client
        .get(format!("{}/time", base_url))
        .timeout(Duration::from_millis(1000))
        .send()
        .await;

    Ok(())
}

/// Warm the POST /order path by making a request that fails fast
/// This ensures subsequent order submissions get ~330ms latency instead of ~900ms
pub async fn prewarm_order_endpoint(client: &Client, base_url: &str) -> Result<(), reqwest::Error> {
    // First warm TCP/TLS with GET
    let _ = client
        .get(format!("{}/time", base_url))
        .timeout(Duration::from_millis(1000))
        .send()
        .await;

    // Then warm POST path - this will fail auth but warms the route
    let _ = client
        .post(format!("{}/order", base_url))
        .header("Content-Type", "application/json")
        .body("{}")
        .timeout(Duration::from_millis(2000))
        .send()
        .await;

    Ok(())
}

/// Create an optimized HTTP client for low-latency trading
/// Benchmarked configuration: 309.3ms vs 349ms baseline (11.4% faster)
pub fn create_optimized_client() -> Result<Client, reqwest::Error> {
    ClientBuilder::new()
        // Connection pooling optimizations - aggressive reuse
        .pool_max_idle_per_host(10) // Keep connections alive
        .pool_idle_timeout(Duration::from_secs(90)) // Longer reuse window
        // TCP optimizations
        .tcp_nodelay(true) // Disable Nagle's algorithm for lower latency
        // HTTP/2 optimizations - empirically tuned
        .http2_adaptive_window(true) // Dynamically adjust flow control
        .http2_initial_stream_window_size(512 * 1024) // 512KB - benchmarked optimal
        // HTTP/2 keep-alive to maintain warm connections indefinitely
        .http2_keep_alive_interval(Duration::from_secs(10))
        .http2_keep_alive_timeout(Duration::from_secs(5))
        .http2_keep_alive_while_idle(true)
        // Compression - all algorithms enabled by default in reqwest
        .gzip(true) // Ensure gzip is enabled
        // User agent for identification
        .user_agent("polyfill-rs/0.1.1 (high-frequency-trading)")
        .build()
}

/// Create a client optimized for co-located environments
/// (even more aggressive settings for when you're close to the exchange)
pub fn create_colocated_client() -> Result<Client, reqwest::Error> {
    ClientBuilder::new()
        // More aggressive connection pooling
        .pool_max_idle_per_host(20) // More connections
        .pool_idle_timeout(Duration::from_secs(60)) // Longer reuse
        // Tighter timeouts for co-located environments
        .connect_timeout(Duration::from_millis(1000)) // 1s connection
        .timeout(Duration::from_millis(10000)) // 10s total
        // TCP optimizations
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        // HTTP/2 with more aggressive keep-alive
        .http2_adaptive_window(true)
        .http2_keep_alive_interval(Duration::from_secs(10))
        .http2_keep_alive_timeout(Duration::from_secs(5))
        .http2_keep_alive_while_idle(true)
        // Disable compression in co-located environments (CPU vs network tradeoff)
        .gzip(false)
        .no_brotli() // Disable brotli compression
        .user_agent("polyfill-rs/0.1.1 (colocated-hft)")
        .build()
}

/// Create a client optimized for high-latency environments
/// (more conservative settings for internet connections)
pub fn create_internet_client() -> Result<Client, reqwest::Error> {
    ClientBuilder::new()
        // Conservative connection pooling
        .pool_max_idle_per_host(5)
        .pool_idle_timeout(Duration::from_secs(90))
        // Longer timeouts for internet connections
        .connect_timeout(Duration::from_millis(10000)) // 10s connection
        .timeout(Duration::from_millis(60000)) // 60s total
        // TCP optimizations
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(120))
        // HTTP/1.1 might be more reliable over internet
        .http1_title_case_headers()
        // Enable compression (gzip and brotli are enabled by default)
        .gzip(true)
        .user_agent("polyfill-rs/0.1.1 (internet-trading)")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimized_client_creation() {
        let client = create_optimized_client();
        assert!(client.is_ok());
    }

    #[test]
    fn test_colocated_client_creation() {
        let client = create_colocated_client();
        assert!(client.is_ok());
    }

    #[test]
    fn test_internet_client_creation() {
        let client = create_internet_client();
        assert!(client.is_ok());
    }
}
