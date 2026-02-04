//! DNS caching to reduce lookup latency
//!
//! This module provides DNS caching functionality to avoid repeated DNS lookups
//! which can add 10-20ms per request.

use hickory_resolver::config::*;
use hickory_resolver::TokioAsyncResolver;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// DNS cache entry with TTL
#[derive(Clone, Debug)]
struct DnsCacheEntry {
    ips: Vec<IpAddr>,
    expires_at: Instant,
}

/// DNS cache for resolving hostnames
pub struct DnsCache {
    resolver: TokioAsyncResolver,
    cache: Arc<RwLock<HashMap<String, DnsCacheEntry>>>,
    default_ttl: Duration,
}

impl DnsCache {
    /// Create a new DNS cache with system configuration
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

        Ok(Self {
            resolver,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: Duration::from_secs(300), // 5 minutes default TTL
        })
    }

    /// Create a DNS cache with custom TTL
    pub async fn with_ttl(ttl: Duration) -> Result<Self, Box<dyn std::error::Error>> {
        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

        Ok(Self {
            resolver,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: ttl,
        })
    }

    /// Resolve a hostname, using cache if available
    pub async fn resolve(&self, hostname: &str) -> Result<Vec<IpAddr>, Box<dyn std::error::Error>> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(hostname) {
                if entry.expires_at > Instant::now() {
                    return Ok(entry.ips.clone());
                }
            }
        }

        // Cache miss or expired, do actual lookup
        let lookup = self.resolver.lookup_ip(hostname).await?;
        let ips: Vec<IpAddr> = lookup.iter().collect();

        // Store in cache
        let entry = DnsCacheEntry {
            ips: ips.clone(),
            expires_at: Instant::now() + self.default_ttl,
        };

        let mut cache = self.cache.write().await;
        cache.insert(hostname.to_string(), entry);

        Ok(ips)
    }

    /// Pre-warm the cache by resolving a hostname
    pub async fn prewarm(&self, hostname: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.resolve(hostname).await?;
        Ok(())
    }

    /// Clear the cache
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Get cache size
    pub async fn cache_size(&self) -> usize {
        let cache = self.cache.read().await;
        cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires external DNS/network access"]
    async fn test_dns_cache_resolve() {
        let cache = DnsCache::new().await.unwrap();
        let ips = cache.resolve("clob.polymarket.com").await.unwrap();
        assert!(!ips.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires external DNS/network access"]
    async fn test_dns_cache_prewarm() {
        let cache = DnsCache::new().await.unwrap();
        cache.prewarm("clob.polymarket.com").await.unwrap();
        assert_eq!(cache.cache_size().await, 1);
    }

    #[tokio::test]
    #[ignore = "requires external DNS/network access"]
    async fn test_dns_cache_clear() {
        let cache = DnsCache::new().await.unwrap();
        cache.prewarm("clob.polymarket.com").await.unwrap();
        cache.clear().await;
        assert_eq!(cache.cache_size().await, 0);
    }
}
