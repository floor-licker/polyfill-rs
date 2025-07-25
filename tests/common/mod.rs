//! Common utilities for integration tests

use polyfill_rs::{ClobClient, Result};
use std::env;
use std::time::Duration;

/// Test configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub host: String,
    pub chain_id: u64,
    pub private_key: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub api_passphrase: Option<String>,
    pub test_timeout: Duration,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            host: env::var("POLYMARKET_HOST").unwrap_or_else(|_| "https://clob.polymarket.com".to_string()),
            chain_id: env::var("POLYMARKET_CHAIN_ID")
                .unwrap_or_else(|_| "137".to_string())
                .parse()
                .unwrap_or(137),
            private_key: env::var("POLYMARKET_PRIVATE_KEY").ok(),
            api_key: env::var("POLYMARKET_API_KEY").ok(),
            api_secret: env::var("POLYMARKET_API_SECRET").ok(),
            api_passphrase: env::var("POLYMARKET_API_PASSPHRASE").ok(),
            test_timeout: Duration::from_secs(30),
        }
    }
}

impl TestConfig {
    /// Check if we have authentication credentials
    pub fn has_auth(&self) -> bool {
        self.private_key.is_some()
    }

    /// Check if we have API credentials
    pub fn has_api_creds(&self) -> bool {
        self.api_key.is_some() && self.api_secret.is_some() && self.api_passphrase.is_some()
    }

    /// Create a basic client for testing
    pub fn create_basic_client(&self) -> ClobClient {
        ClobClient::new(&self.host)
    }

    /// Create an authenticated client for testing
    pub fn create_auth_client(&self) -> Result<ClobClient> {
        let private_key = self.private_key.as_ref()
            .ok_or_else(|| polyfill_rs::PolyfillError::auth("No private key provided", polyfill_rs::errors::AuthErrorKind::InvalidCredentials))?;
        
        Ok(ClobClient::with_l1_headers(&self.host, private_key, self.chain_id))
    }

    /// Print test configuration (without sensitive data)
    pub fn print_config(&self) {
        println!("Test Configuration:");
        println!("  Host: {}", self.host);
        println!("  Chain ID: {}", self.chain_id);
        println!("  Has Auth: {}", self.has_auth());
        println!("  Has API Creds: {}", self.has_api_creds());
        println!("  Timeout: {:?}", self.test_timeout);
    }
}

/// Test utilities for common operations
pub struct TestUtils;

impl TestUtils {
    /// Get a valid token_id for testing
    pub async fn get_test_token_id(client: &ClobClient) -> Result<String> {
        let markets = client.get_sampling_markets(None).await?;
        if markets.data.is_empty() {
            return Err(polyfill_rs::PolyfillError::internal_simple("No markets available for testing"));
        }
        
        let token_id = markets.data[0].tokens[0].token_id.clone();
        println!("Using test token_id: {}", token_id);
        Ok(token_id)
    }

    /// Wait for a condition with timeout
    pub async fn wait_for<F, Fut>(mut condition: F, timeout: Duration) -> Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<bool>>,
    {
        let start = std::time::Instant::now();
        let check_interval = Duration::from_millis(100);

        while start.elapsed() < timeout {
            if condition().await? {
                return Ok(());
            }
            tokio::time::sleep(check_interval).await;
        }

        Err(polyfill_rs::PolyfillError::timeout(
            timeout,
            "Condition not met within timeout".to_string(),
        ))
    }

    /// Measure execution time of an async operation
    pub async fn measure_time<F, Fut, T>(operation: F) -> (T, Duration)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let start = std::time::Instant::now();
        let result = operation().await;
        let duration = start.elapsed();
        (result, duration)
    }

    /// Assert that an operation completes within a reasonable time
    pub async fn assert_timely<F, Fut, T>(operation: F, max_duration: Duration) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let (result, duration) = Self::measure_time(|| async {
            operation().await
        }).await;

        if duration > max_duration {
            return Err(polyfill_rs::PolyfillError::timeout(
                duration,
                format!("Operation took too long: {:?} > {:?}", duration, max_duration),
            ));
        }

        println!("Operation completed in {:?}", duration);
        result
    }
}

/// Test result reporting
pub struct TestReporter;

impl TestReporter {
    /// Report test success
    pub fn success(test_name: &str) {
        println!("✅ {} passed", test_name);
    }

    /// Report test failure
    pub fn failure(test_name: &str, error: &dyn std::error::Error) {
        println!("❌ {} failed: {}", test_name, error);
    }

    /// Report test skip
    pub fn skip(test_name: &str, reason: &str) {
        println!("⚠️  {} skipped: {}", test_name, reason);
    }

    /// Report test performance
    pub fn performance(test_name: &str, duration: Duration) {
        println!("⚡ {} completed in {:?}", test_name, duration);
    }
} 