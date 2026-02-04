//! Connection management for maintaining warm HTTP connections
//!
//! This module provides functionality to keep connections alive and prevent
//! connection drops that cause 200ms+ reconnection overhead.

use reqwest::Client;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Connection keep-alive manager
pub struct ConnectionManager {
    client: Client,
    base_url: String,
    running: Arc<AtomicBool>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new(client: Client, base_url: String) -> Self {
        Self {
            client,
            base_url,
            running: Arc::new(AtomicBool::new(false)),
            handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the keep-alive background task
    /// Sends periodic lightweight requests to keep the connection warm
    pub async fn start_keepalive(&self, interval: Duration) {
        // If already running, return
        if self.running.load(Ordering::Relaxed) {
            return;
        }

        self.running.store(true, Ordering::Relaxed);

        let client = self.client.clone();
        let base_url = self.base_url.clone();
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                // Send a lightweight request to keep connection alive
                // Use /time endpoint as it's fast and doesn't require auth
                let _ = client
                    .get(format!("{}/time", base_url))
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await;

                // Wait for next interval
                tokio::time::sleep(interval).await;
            }
        });

        let mut handle_guard = self.handle.lock().await;
        *handle_guard = Some(handle);
    }

    /// Stop the keep-alive background task
    pub async fn stop_keepalive(&self) {
        self.running.store(false, Ordering::Relaxed);

        let mut handle_guard = self.handle.lock().await;
        if let Some(handle) = handle_guard.take() {
            handle.abort();
        }
    }

    /// Check if keep-alive is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Send a single keep-alive ping
    pub async fn ping(&self) -> Result<(), reqwest::Error> {
        self.client
            .get(format!("{}/time", self.base_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await?;
        Ok(())
    }
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_manager_creation() {
        let client = reqwest::ClientBuilder::new().no_proxy().build().unwrap();
        let manager = ConnectionManager::new(client, "https://clob.polymarket.com".to_string());
        assert!(!manager.is_running());
    }

    #[tokio::test]
    async fn test_keepalive_start_stop() {
        let client = reqwest::ClientBuilder::new().no_proxy().build().unwrap();
        let manager = ConnectionManager::new(client, "https://clob.polymarket.com".to_string());

        manager.start_keepalive(Duration::from_secs(30)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(manager.is_running());

        manager.stop_keepalive().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!manager.is_running());
    }
}
