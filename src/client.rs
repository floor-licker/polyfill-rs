//! High-performance Rust client for Polymarket
//!
//! This module provides a production-ready client for interacting with
//! Polymarket, optimized for high-frequency trading environments.

use crate::auth::{create_l1_headers, create_l2_headers};
use crate::errors::{PolyfillError, Result};
use crate::http_config::{
    create_colocated_client, create_internet_client, create_optimized_client, prewarm_connections,
};
use crate::types::{OrderOptions, PostOrder, SignedOrderRequest};
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use reqwest::header::HeaderName;
use reqwest::Client;
use reqwest::{Method, RequestBuilder};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

// Re-export types for compatibility
pub use crate::types::{ApiCredentials as ApiCreds, OrderType, Side};

// Compatibility types
#[derive(Debug)]
pub struct OrderArgs {
    pub token_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub side: Side,
}

impl OrderArgs {
    pub fn new(token_id: &str, price: Decimal, size: Decimal, side: Side) -> Self {
        Self {
            token_id: token_id.to_string(),
            price,
            size,
            side,
        }
    }
}

impl Default for OrderArgs {
    fn default() -> Self {
        Self {
            token_id: "".to_string(),
            price: Decimal::ZERO,
            size: Decimal::ZERO,
            side: Side::BUY,
        }
    }
}

/// Main client for interacting with Polymarket API
pub struct ClobClient {
    pub http_client: Client,
    pub base_url: String,
    chain_id: u64,
    signer: Option<PrivateKeySigner>,
    api_creds: Option<ApiCreds>,
    order_builder: Option<crate::orders::OrderBuilder>,
    #[allow(dead_code)]
    dns_cache: Option<std::sync::Arc<crate::dns_cache::DnsCache>>,
    #[allow(dead_code)]
    connection_manager: Option<std::sync::Arc<crate::connection_manager::ConnectionManager>>,
    #[allow(dead_code)]
    buffer_pool: std::sync::Arc<crate::buffer_pool::BufferPool>,
}

impl ClobClient {
    /// Create a new client with optimized HTTP/2 settings (benchmarked 11.4% faster)
    /// Now includes DNS caching, connection management, and buffer pooling
    pub fn new(host: &str) -> Self {
        // Benchmarked optimal configuration: 512KB stream window
        // Results: 309.3ms vs 349ms baseline (11.4% improvement)
        let optimized_client = reqwest::ClientBuilder::new()
            // Avoid reading OS proxy settings (can be slow and/or unavailable in some sandboxed envs)
            .no_proxy()
            .http2_adaptive_window(true)
            .http2_initial_stream_window_size(512 * 1024) // 512KB - empirically optimal
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| {
                reqwest::ClientBuilder::new()
                    .no_proxy()
                    .build()
                    .expect("Failed to build reqwest client")
            });

        // Initialize DNS cache and pre-warm it
        let dns_cache = tokio::runtime::Handle::try_current().ok().and_then(|_| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let cache = crate::dns_cache::DnsCache::new().await.ok()?;
                    let hostname = host
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .split('/')
                        .next()?;
                    cache.prewarm(hostname).await.ok()?;
                    Some(std::sync::Arc::new(cache))
                })
            })
        });

        // Initialize connection manager
        let connection_manager = Some(std::sync::Arc::new(
            crate::connection_manager::ConnectionManager::new(
                optimized_client.clone(),
                host.to_string(),
            ),
        ));

        // Initialize buffer pool (512KB buffers, pool of 10)
        let buffer_pool = std::sync::Arc::new(crate::buffer_pool::BufferPool::new(512 * 1024, 10));

        // Pre-warm buffer pool with 3 buffers
        let pool_clone = buffer_pool.clone();
        if let Ok(_handle) = tokio::runtime::Handle::try_current() {
            tokio::spawn(async move {
                pool_clone.prewarm(3).await;
            });
        }

        Self {
            http_client: optimized_client,
            base_url: host.to_string(),
            chain_id: 137, // Default to Polygon
            signer: None,
            api_creds: None,
            order_builder: None,
            dns_cache,
            connection_manager,
            buffer_pool,
        }
    }

    /// Create a client optimized for co-located environments
    pub fn new_colocated(host: &str) -> Self {
        let http_client = create_colocated_client().unwrap_or_else(|_| {
            reqwest::ClientBuilder::new()
                .no_proxy()
                .build()
                .expect("Failed to build reqwest client")
        });

        let connection_manager = Some(std::sync::Arc::new(
            crate::connection_manager::ConnectionManager::new(
                http_client.clone(),
                host.to_string(),
            ),
        ));
        let buffer_pool = std::sync::Arc::new(crate::buffer_pool::BufferPool::new(512 * 1024, 10));

        Self {
            http_client,
            base_url: host.to_string(),
            chain_id: 137,
            signer: None,
            api_creds: None,
            order_builder: None,
            dns_cache: None,
            connection_manager,
            buffer_pool,
        }
    }

    /// Create a client optimized for internet connections
    pub fn new_internet(host: &str) -> Self {
        let http_client = create_internet_client().unwrap_or_else(|_| {
            reqwest::ClientBuilder::new()
                .no_proxy()
                .build()
                .expect("Failed to build reqwest client")
        });

        let connection_manager = Some(std::sync::Arc::new(
            crate::connection_manager::ConnectionManager::new(
                http_client.clone(),
                host.to_string(),
            ),
        ));
        let buffer_pool = std::sync::Arc::new(crate::buffer_pool::BufferPool::new(512 * 1024, 10));

        Self {
            http_client,
            base_url: host.to_string(),
            chain_id: 137,
            signer: None,
            api_creds: None,
            order_builder: None,
            dns_cache: None,
            connection_manager,
            buffer_pool,
        }
    }

    /// Create a client with L1 headers (for authentication)
    pub fn with_l1_headers(host: &str, private_key: &str, chain_id: u64) -> Self {
        let signer = private_key
            .parse::<PrivateKeySigner>()
            .expect("Invalid private key");

        let order_builder = crate::orders::OrderBuilder::new(signer.clone(), None, None);

        let http_client = create_optimized_client().unwrap_or_else(|_| {
            reqwest::ClientBuilder::new()
                .no_proxy()
                .build()
                .expect("Failed to build reqwest client")
        });

        // Initialize infrastructure modules
        let dns_cache = None; // Skip DNS cache for simplicity in this constructor
        let connection_manager = Some(std::sync::Arc::new(
            crate::connection_manager::ConnectionManager::new(
                http_client.clone(),
                host.to_string(),
            ),
        ));
        let buffer_pool = std::sync::Arc::new(crate::buffer_pool::BufferPool::new(512 * 1024, 10));

        Self {
            http_client,
            base_url: host.to_string(),
            chain_id,
            signer: Some(signer),
            api_creds: None,
            order_builder: Some(order_builder),
            dns_cache,
            connection_manager,
            buffer_pool,
        }
    }

    /// Create a client with L2 headers (for API key authentication)
    pub fn with_l2_headers(
        host: &str,
        private_key: &str,
        chain_id: u64,
        api_creds: ApiCreds,
    ) -> Self {
        let signer = private_key
            .parse::<PrivateKeySigner>()
            .expect("Invalid private key");

        let order_builder = crate::orders::OrderBuilder::new(signer.clone(), None, None);

        let http_client = create_optimized_client().unwrap_or_else(|_| {
            reqwest::ClientBuilder::new()
                .no_proxy()
                .build()
                .expect("Failed to build reqwest client")
        });

        // Initialize infrastructure modules
        let dns_cache = None; // Skip DNS cache for simplicity in this constructor
        let connection_manager = Some(std::sync::Arc::new(
            crate::connection_manager::ConnectionManager::new(
                http_client.clone(),
                host.to_string(),
            ),
        ));
        let buffer_pool = std::sync::Arc::new(crate::buffer_pool::BufferPool::new(512 * 1024, 10));

        Self {
            http_client,
            base_url: host.to_string(),
            chain_id,
            signer: Some(signer),
            api_creds: Some(api_creds),
            order_builder: Some(order_builder),
            dns_cache,
            connection_manager,
            buffer_pool,
        }
    }

    /// Set API credentials
    pub fn set_api_creds(&mut self, api_creds: ApiCreds) {
        self.api_creds = Some(api_creds);
    }

    /// Start background keep-alive to maintain warm connection
    /// Sends periodic lightweight requests to prevent connection drops
    pub async fn start_keepalive(&self, interval: std::time::Duration) {
        if let Some(manager) = &self.connection_manager {
            manager.start_keepalive(interval).await;
        }
    }

    /// Stop keep-alive background task
    pub async fn stop_keepalive(&self) {
        if let Some(manager) = &self.connection_manager {
            manager.stop_keepalive().await;
        }
    }

    /// Pre-warm connections to reduce first-request latency
    pub async fn prewarm_connections(&self) -> Result<()> {
        prewarm_connections(&self.http_client, &self.base_url)
            .await
            .map_err(|e| {
                PolyfillError::network(format!("Failed to prewarm connections: {}", e), e)
            })?;
        Ok(())
    }

    /// Get the wallet address
    pub fn get_address(&self) -> Option<String> {
        use alloy_primitives::hex;
        self.signer
            .as_ref()
            .map(|s| hex::encode_prefixed(s.address().as_slice()))
    }

    /// Get the collateral token address for the current chain
    pub fn get_collateral_address(&self) -> Option<String> {
        let config = crate::orders::get_contract_config(self.chain_id, false)?;
        Some(config.collateral)
    }

    /// Get the conditional tokens contract address for the current chain
    pub fn get_conditional_address(&self) -> Option<String> {
        let config = crate::orders::get_contract_config(self.chain_id, false)?;
        Some(config.conditional_tokens)
    }

    /// Get the exchange contract address for the current chain
    pub fn get_exchange_address(&self) -> Option<String> {
        let config = crate::orders::get_contract_config(self.chain_id, false)?;
        Some(config.exchange)
    }

    /// Test basic connectivity
    pub async fn get_ok(&self) -> bool {
        match self
            .http_client
            .get(format!("{}/ok", self.base_url))
            .send()
            .await
        {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    /// Get server time
    pub async fn get_server_time(&self) -> Result<u64> {
        let response = self
            .http_client
            .get(format!("{}/time", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get server time",
            ));
        }

        let time_text = response.text().await?;
        let timestamp = time_text
            .trim()
            .parse::<u64>()
            .map_err(|e| PolyfillError::parse(format!("Invalid timestamp format: {}", e), None))?;

        Ok(timestamp)
    }

    /// Get order book for a token
    pub async fn get_order_book(&self, token_id: &str) -> Result<OrderBookSummary> {
        let response = self
            .http_client
            .get(format!("{}/book", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get order book",
            ));
        }

        let order_book: OrderBookSummary = response.json().await?;
        Ok(order_book)
    }

    /// Get midpoint for a token
    pub async fn get_midpoint(&self, token_id: &str) -> Result<MidpointResponse> {
        let response = self
            .http_client
            .get(format!("{}/midpoint", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get midpoint",
            ));
        }

        let midpoint: MidpointResponse = response.json().await?;
        Ok(midpoint)
    }

    /// Get spread for a token
    pub async fn get_spread(&self, token_id: &str) -> Result<SpreadResponse> {
        let response = self
            .http_client
            .get(format!("{}/spread", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get spread",
            ));
        }

        let spread: SpreadResponse = response.json().await?;
        Ok(spread)
    }

    /// Get spreads for multiple tokens (batch)
    pub async fn get_spreads(
        &self,
        token_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Decimal>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(format!("{}/spreads", self.base_url))
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get batch spreads",
            ));
        }

        response
            .json::<std::collections::HashMap<String, Decimal>>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get price for a token and side
    pub async fn get_price(&self, token_id: &str, side: Side) -> Result<PriceResponse> {
        let response = self
            .http_client
            .get(format!("{}/price", self.base_url))
            .query(&[("token_id", token_id), ("side", side.as_str())])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get price",
            ));
        }

        let price: PriceResponse = response.json().await?;
        Ok(price)
    }

    fn validate_prices_history_asset_id(asset_id: &str) -> Result<()> {
        if asset_id.is_empty() {
            return Err(PolyfillError::validation(
                "asset_id is required (use the decimal token_id / asset_id)",
            ));
        }

        // Common footgun: passing a condition id (0x...) instead of the decimal asset id.
        if asset_id.starts_with("0x") || asset_id.starts_with("0X") {
            return Err(PolyfillError::validation(
                "`/prices-history` expects a decimal token_id/asset_id, not a hex condition_id",
            ));
        }

        if !asset_id.as_bytes().iter().all(u8::is_ascii_digit) {
            return Err(PolyfillError::validation(
                "asset_id must be a decimal string (token_id / asset_id)",
            ));
        }

        Ok(())
    }

    /// Get price history for a single outcome (`token_id` / `asset_id`) over a fixed interval.
    ///
    /// Important: the upstream API query parameter is named `market`, but it expects the
    /// decimal outcome asset id (not the hex `condition_id`).
    pub async fn get_prices_history_interval(
        &self,
        asset_id: &str,
        interval: PricesHistoryInterval,
        fidelity: Option<u32>,
    ) -> Result<PricesHistoryResponse> {
        Self::validate_prices_history_asset_id(asset_id)?;

        let mut request = self
            .http_client
            .get(format!("{}/prices-history", self.base_url))
            .query(&[("market", asset_id), ("interval", interval.as_str())]);

        if let Some(fidelity) = fidelity {
            request = request.query(&[("fidelity", fidelity)]);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| {
                    if body.is_empty() {
                        "Failed to get prices history".to_string()
                    } else {
                        body
                    }
                });
            return Err(PolyfillError::api(status, message));
        }

        Ok(response.json::<PricesHistoryResponse>().await?)
    }

    /// Get price history for a single outcome (`token_id` / `asset_id`) over a timestamp range.
    ///
    /// `start_ts` and `end_ts` are Unix timestamps (seconds).
    pub async fn get_prices_history_range(
        &self,
        asset_id: &str,
        start_ts: u64,
        end_ts: u64,
        fidelity: Option<u32>,
    ) -> Result<PricesHistoryResponse> {
        Self::validate_prices_history_asset_id(asset_id)?;

        if start_ts >= end_ts {
            return Err(PolyfillError::validation(
                "start_ts must be < end_ts for prices history",
            ));
        }

        let mut request = self
            .http_client
            .get(format!("{}/prices-history", self.base_url))
            .query(&[("market", asset_id)])
            .query(&[("startTs", start_ts), ("endTs", end_ts)]);

        if let Some(fidelity) = fidelity {
            request = request.query(&[("fidelity", fidelity)]);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| {
                    if body.is_empty() {
                        "Failed to get prices history".to_string()
                    } else {
                        body
                    }
                });
            return Err(PolyfillError::api(status, message));
        }

        Ok(response.json::<PricesHistoryResponse>().await?)
    }

    /// Get tick size for a token
    pub async fn get_tick_size(&self, token_id: &str) -> Result<Decimal> {
        let response = self
            .http_client
            .get(format!("{}/tick-size", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get tick size",
            ));
        }

        let tick_size_response: Value = response.json().await?;
        let tick_size = tick_size_response["minimum_tick_size"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .or_else(|| {
                tick_size_response["minimum_tick_size"]
                    .as_f64()
                    .map(|f| Decimal::from_f64(f).unwrap_or(Decimal::ZERO))
            })
            .ok_or_else(|| PolyfillError::parse("Invalid tick size format", None))?;

        Ok(tick_size)
    }

    /// Get maker fee rate (in bps) for a token
    pub async fn get_fee_rate_bps(&self, token_id: &str) -> Result<u32> {
        let response = self
            .http_client
            .get(format!("{}/fee-rate", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get fee rate",
            ));
        }

        let fee_rate: crate::types::FeeRateResponse = response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))?;
        Ok(fee_rate.fee_rate_bps)
    }

    /// Create a new API key
    pub async fn create_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;

        let headers = create_l1_headers(signer, nonce)?;
        let req =
            self.create_request_with_headers(Method::POST, "/auth/api-key", headers.into_iter());

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to create API key",
            ));
        }

        Ok(response.json::<ApiCreds>().await?)
    }

    /// Derive an existing API key
    pub async fn derive_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;

        let headers = create_l1_headers(signer, nonce)?;
        let req = self.create_request_with_headers(
            Method::GET,
            "/auth/derive-api-key",
            headers.into_iter(),
        );

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to derive API key",
            ));
        }

        Ok(response.json::<ApiCreds>().await?)
    }

    /// Create or derive API key (try create first, fallback to derive)
    pub async fn create_or_derive_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        match self.create_api_key(nonce).await {
            Ok(creds) => Ok(creds),
            // Only fall back to derive on API status errors (server responded).
            // Propagate network/parse/internal errors so callers can handle them appropriately.
            Err(PolyfillError::Api { .. }) => self.derive_api_key(nonce).await,
            Err(err) => Err(err),
        }
    }

    /// Get all API keys for the authenticated user
    pub async fn get_api_keys(&self) -> Result<Vec<String>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::GET;
        let endpoint = "/auth/api-keys";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        let api_keys_response: crate::types::ApiKeysResponse = response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))?;

        Ok(api_keys_response.api_keys)
    }

    /// Delete the current API key
    pub async fn delete_api_key(&self) -> Result<String> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::DELETE;
        let endpoint = "/auth/api-key";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .text()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Helper to create request with headers
    fn create_request_with_headers(
        &self,
        method: Method,
        endpoint: &str,
        headers: impl Iterator<Item = (&'static str, String)>,
    ) -> RequestBuilder {
        let req = self
            .http_client
            .request(method, format!("{}{}", &self.base_url, endpoint));
        headers.fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v))
    }

    /// Get neg risk for a token
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        let response = self
            .http_client
            .get(format!("{}/neg-risk", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get neg risk",
            ));
        }

        let neg_risk_response: Value = response.json().await?;
        let neg_risk = neg_risk_response["neg_risk"]
            .as_bool()
            .ok_or_else(|| PolyfillError::parse("Invalid neg risk format", None))?;

        Ok(neg_risk)
    }

    /// Resolve tick size for an order
    async fn resolve_tick_size(
        &self,
        token_id: &str,
        tick_size: Option<Decimal>,
    ) -> Result<Decimal> {
        let min_tick_size = self.get_tick_size(token_id).await?;

        match tick_size {
            None => Ok(min_tick_size),
            Some(t) => {
                if t < min_tick_size {
                    Err(PolyfillError::validation(format!(
                        "Tick size {} is smaller than min_tick_size {} for token_id: {}",
                        t, min_tick_size, token_id
                    )))
                } else {
                    Ok(t)
                }
            },
        }
    }

    /// Get filled order options
    async fn get_filled_order_options(
        &self,
        token_id: &str,
        options: Option<&OrderOptions>,
    ) -> Result<OrderOptions> {
        let (tick_size, neg_risk, fee_rate_bps) = match options {
            Some(o) => (o.tick_size, o.neg_risk, o.fee_rate_bps),
            None => (None, None, None),
        };

        let tick_size = self.resolve_tick_size(token_id, tick_size).await?;
        let neg_risk = match neg_risk {
            Some(nr) => nr,
            None => self.get_neg_risk(token_id).await?,
        };

        Ok(OrderOptions {
            tick_size: Some(tick_size),
            neg_risk: Some(neg_risk),
            fee_rate_bps,
        })
    }

    /// Check if price is in valid range
    fn is_price_in_range(&self, price: Decimal, tick_size: Decimal) -> bool {
        let min_price = tick_size;
        let max_price = Decimal::ONE - tick_size;
        price >= min_price && price <= max_price
    }

    /// Create an order
    pub async fn create_order(
        &self,
        order_args: &OrderArgs,
        expiration: Option<u64>,
        extras: Option<crate::types::ExtraOrderArgs>,
        options: Option<&OrderOptions>,
    ) -> Result<SignedOrderRequest> {
        let order_builder = self
            .order_builder
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Order builder not initialized"))?;

        let create_order_options = self
            .get_filled_order_options(&order_args.token_id, options)
            .await?;

        let expiration = expiration.unwrap_or(0);
        let extras = extras.unwrap_or_default();

        if !self.is_price_in_range(
            order_args.price,
            create_order_options.tick_size.expect("Should be filled"),
        ) {
            return Err(PolyfillError::validation(
                "Price is not in range of tick_size",
            ));
        }

        order_builder.create_order(
            self.chain_id,
            order_args,
            expiration,
            &extras,
            &create_order_options,
        )
    }

    /// Calculate market price from order book
    async fn calculate_market_price(
        &self,
        token_id: &str,
        side: Side,
        amount: Decimal,
    ) -> Result<Decimal> {
        let book = self.get_order_book(token_id).await?;
        let order_builder = self
            .order_builder
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Order builder not initialized"))?;

        // Convert OrderSummary to BookLevel
        let levels: Vec<crate::types::BookLevel> = match side {
            Side::BUY => book
                .asks
                .into_iter()
                .map(|s| crate::types::BookLevel {
                    price: s.price,
                    size: s.size,
                })
                .collect(),
            Side::SELL => book
                .bids
                .into_iter()
                .map(|s| crate::types::BookLevel {
                    price: s.price,
                    size: s.size,
                })
                .collect(),
        };

        order_builder.calculate_market_price(&levels, amount)
    }

    /// Create a market order
    pub async fn create_market_order(
        &self,
        order_args: &crate::types::MarketOrderArgs,
        extras: Option<crate::types::ExtraOrderArgs>,
        options: Option<&OrderOptions>,
    ) -> Result<SignedOrderRequest> {
        let order_builder = self
            .order_builder
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Order builder not initialized"))?;

        let create_order_options = self
            .get_filled_order_options(&order_args.token_id, options)
            .await?;

        let extras = extras.unwrap_or_default();
        let price = self
            .calculate_market_price(&order_args.token_id, Side::BUY, order_args.amount)
            .await?;

        if !self.is_price_in_range(
            price,
            create_order_options.tick_size.expect("Should be filled"),
        ) {
            return Err(PolyfillError::validation(
                "Price is not in range of tick_size",
            ));
        }

        order_builder.create_market_order(
            self.chain_id,
            order_args,
            price,
            &extras,
            &create_order_options,
        )
    }

    /// Post an order to the exchange
    pub async fn post_order(
        &self,
        order: SignedOrderRequest,
        order_type: OrderType,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        // Owner field must reference the credential principal identifier
        // to maintain consistency with the authentication context layer
        let body = PostOrder::new(order, api_creds.api_key.clone(), order_type);

        let headers = create_l2_headers(signer, api_creds, "POST", "/order", Some(&body))?;
        let req = self.create_request_with_headers(Method::POST, "/order", headers.into_iter());

        let response = req.json(&body).send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            let message = if body.is_empty() {
                "Failed to post order".to_string()
            } else {
                format!("Failed to post order: {}", body)
            };
            return Err(PolyfillError::api(status, message));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Create and post an order in one call
    pub async fn create_and_post_order(&self, order_args: &OrderArgs) -> Result<Value> {
        let order = self.create_order(order_args, None, None, None).await?;
        self.post_order(order, OrderType::GTC).await
    }

    /// Cancel an order
    pub async fn cancel(&self, order_id: &str) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let body = std::collections::HashMap::from([("orderID", order_id)]);

        let headers = create_l2_headers(signer, api_creds, "DELETE", "/order", Some(&body))?;
        let req = self.create_request_with_headers(Method::DELETE, "/order", headers.into_iter());

        let response = req.json(&body).send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to cancel order",
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Cancel multiple orders
    pub async fn cancel_orders(&self, order_ids: &[String]) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers(signer, api_creds, "DELETE", "/orders", Some(order_ids))?;
        let req = self.create_request_with_headers(Method::DELETE, "/orders", headers.into_iter());

        let response = req.json(order_ids).send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to cancel orders",
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Cancel all orders
    pub async fn cancel_all(&self) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers::<Value>(signer, api_creds, "DELETE", "/cancel-all", None)?;
        let req =
            self.create_request_with_headers(Method::DELETE, "/cancel-all", headers.into_iter());

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to cancel all orders",
            ));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Get open orders with optional filtering
    ///
    /// This retrieves all open orders for the authenticated user. You can filter by:
    /// - Order ID (exact match)
    /// - Asset/Token ID (all orders for a specific token)
    /// - Market ID (all orders for a specific market)
    ///
    /// The response includes order status, fill information, and timestamps.
    pub async fn get_orders(
        &self,
        params: Option<&crate::types::OpenOrderParams>,
        next_cursor: Option<&str>,
    ) -> Result<Vec<crate::types::OpenOrder>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/data/orders";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = match params {
            None => Vec::new(),
            Some(p) => p.to_query_params(),
        };

        let mut next_cursor = next_cursor.unwrap_or("MA==").to_string(); // INITIAL_CURSOR
        let mut output = Vec::new();

        while next_cursor != "LTE=" {
            // END_CURSOR
            let req = self
                .http_client
                .request(method.clone(), format!("{}{}", self.base_url, endpoint))
                .query(&query_params)
                .query(&[("next_cursor", &next_cursor)]);

            let r = headers
                .clone()
                .into_iter()
                .fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v));

            let resp = r
                .send()
                .await
                .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?
                .json::<Value>()
                .await
                .map_err(|e| {
                    PolyfillError::parse(format!("Failed to parse response: {}", e), None)
                })?;

            let new_cursor = resp["next_cursor"]
                .as_str()
                .ok_or_else(|| {
                    PolyfillError::parse("Failed to parse next cursor".to_string(), None)
                })?
                .to_owned();

            next_cursor = new_cursor;

            let results = resp["data"].clone();
            let orders =
                serde_json::from_value::<Vec<crate::types::OpenOrder>>(results).map_err(|e| {
                    PolyfillError::parse(
                        format!("Failed to parse data from order response: {}", e),
                        None,
                    )
                })?;
            output.extend(orders);
        }

        Ok(output)
    }

    /// Get trade history with optional filtering
    ///
    /// This retrieves historical trades for the authenticated user. You can filter by:
    /// - Trade ID (exact match)
    /// - Maker address (trades where you were the maker)
    /// - Market ID (trades in a specific market)
    /// - Asset/Token ID (trades for a specific token)
    /// - Time range (before/after timestamps)
    ///
    /// Trades are returned in reverse chronological order (newest first).
    pub async fn get_trades(
        &self,
        trade_params: Option<&crate::types::TradeParams>,
        next_cursor: Option<&str>,
    ) -> Result<Vec<Value>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/data/trades";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = match trade_params {
            None => Vec::new(),
            Some(p) => p.to_query_params(),
        };

        let mut next_cursor = next_cursor.unwrap_or("MA==").to_string(); // INITIAL_CURSOR
        let mut output = Vec::new();

        while next_cursor != "LTE=" {
            // END_CURSOR
            let req = self
                .http_client
                .request(method.clone(), format!("{}{}", self.base_url, endpoint))
                .query(&query_params)
                .query(&[("next_cursor", &next_cursor)]);

            let r = headers
                .clone()
                .into_iter()
                .fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v));

            let resp = r
                .send()
                .await
                .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?
                .json::<Value>()
                .await
                .map_err(|e| {
                    PolyfillError::parse(format!("Failed to parse response: {}", e), None)
                })?;

            let new_cursor = resp["next_cursor"]
                .as_str()
                .ok_or_else(|| {
                    PolyfillError::parse("Failed to parse next cursor".to_string(), None)
                })?
                .to_owned();

            next_cursor = new_cursor;

            let results = resp["data"].clone();
            output.push(results);
        }

        Ok(output)
    }

    /// Get balance and allowance information for all assets
    ///
    /// This returns the current balance and allowance for each asset in your account.
    /// Balance is how much you own, allowance is how much the exchange can spend on your behalf.
    ///
    /// You need both balance and allowance to place orders - the exchange needs permission
    /// to move your tokens when orders are filled.
    pub async fn get_balance_allowance(
        &self,
        params: Option<crate::types::BalanceAllowanceParams>,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let mut params = params.unwrap_or_default();
        if params.signature_type.is_none() {
            params.set_signature_type(
                self.order_builder
                    .as_ref()
                    .expect("OrderBuilder not set")
                    .get_sig_type(),
            );
        }

        let query_params = params.to_query_params();

        let method = Method::GET;
        let endpoint = "/balance-allowance";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Set up notifications for order fills and other events
    ///
    /// This configures push notifications so you get alerted when:
    /// - Your orders get filled
    /// - Your orders get cancelled
    /// - Market conditions change significantly
    ///
    /// The signature proves you own the account and want to receive notifications.
    pub async fn get_notifications(&self) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/notifications";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&[(
                "signature_type",
                &self
                    .order_builder
                    .as_ref()
                    .expect("OrderBuilder not set")
                    .get_sig_type()
                    .to_string(),
            )])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get midpoints for multiple tokens in a single request
    ///
    /// This is much more efficient than calling get_midpoint() multiple times.
    /// Instead of N round trips, you make just 1 request and get all the midpoints back.
    ///
    /// Midpoints are returned as a HashMap where the key is the token_id and the value
    /// is the midpoint price (or None if there's no valid midpoint).
    pub async fn get_midpoints(
        &self,
        token_ids: &[String],
    ) -> Result<std::collections::HashMap<String, Decimal>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(format!("{}/midpoints", self.base_url))
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get batch midpoints",
            ));
        }

        let midpoints: std::collections::HashMap<String, Decimal> = response.json().await?;
        Ok(midpoints)
    }

    /// Get bid/ask/mid prices for multiple tokens in a single request
    ///
    /// This gives you the full price picture for multiple tokens at once.
    /// Much more efficient than individual calls, especially when you're tracking
    /// a portfolio or comparing multiple markets.
    ///
    /// Returns bid (best buy price), ask (best sell price), and mid (average) for each token.
    pub async fn get_prices(
        &self,
        book_params: &[crate::types::BookParams],
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<Side, Decimal>>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = book_params
            .iter()
            .map(|params| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", params.token_id.clone());
                map.insert("side", params.side.as_str().to_string());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(format!("{}/prices", self.base_url))
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get batch prices",
            ));
        }

        let prices: std::collections::HashMap<String, std::collections::HashMap<Side, Decimal>> =
            response.json().await?;
        Ok(prices)
    }

    /// Get order book for multiple tokens (batch) - reference implementation compatible
    pub async fn get_order_books(&self, token_ids: &[String]) -> Result<Vec<OrderBookSummary>> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(format!("{}/books", self.base_url))
            .json(&request_data)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Vec<OrderBookSummary>>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get single order by ID
    pub async fn get_order(&self, order_id: &str) -> Result<crate::types::OpenOrder> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::GET;
        let endpoint = &format!("/data/order/{}", order_id);
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::OpenOrder>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get last trade price for a token
    pub async fn get_last_trade_price(&self, token_id: &str) -> Result<Value> {
        let response = self
            .http_client
            .get(format!("{}/last-trade-price", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get last trade prices for multiple tokens
    pub async fn get_last_trade_prices(&self, token_ids: &[String]) -> Result<Value> {
        let request_data: Vec<std::collections::HashMap<&str, String>> = token_ids
            .iter()
            .map(|id| {
                let mut map = std::collections::HashMap::new();
                map.insert("token_id", id.clone());
                map
            })
            .collect();

        let response = self
            .http_client
            .post(format!("{}/last-trades-prices", self.base_url))
            .json(&request_data)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Cancel market orders with optional filters
    pub async fn cancel_market_orders(
        &self,
        market: Option<&str>,
        asset_id: Option<&str>,
    ) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::DELETE;
        let endpoint = "/cancel-market-orders";
        let body = std::collections::HashMap::from([
            ("market", market.unwrap_or("")),
            ("asset_id", asset_id.unwrap_or("")),
        ]);

        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(&body))?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Drop (delete) notifications by IDs
    pub async fn drop_notifications(&self, ids: &[String]) -> Result<Value> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::DELETE;
        let endpoint = "/notifications";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&[("ids", ids.join(","))])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Update balance allowance
    pub async fn update_balance_allowance(
        &self,
        params: Option<crate::types::BalanceAllowanceParams>,
    ) -> Result<()> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let mut params = params.unwrap_or_default();
        if params.signature_type.is_none() {
            params.set_signature_type(
                self.order_builder
                    .as_ref()
                    .expect("OrderBuilder not set")
                    .get_sig_type(),
            );
        }

        let query_params = params.to_query_params();

        let method = Method::GET;
        let endpoint = "/balance-allowance/update";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        self.http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        Ok(())
    }

    /// Check if an order is scoring
    pub async fn is_order_scoring(&self, order_id: &str) -> Result<bool> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::GET;
        let endpoint = "/order-scoring";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .query(&[("order_id", order_id)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        let result: Value = response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))?;

        Ok(result["scoring"].as_bool().unwrap_or(false))
    }

    /// Check if multiple orders are scoring
    pub async fn are_orders_scoring(
        &self,
        order_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, bool>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::config("Signer not configured"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::config("API credentials not configured"))?;

        let method = Method::POST;
        let endpoint = "/orders-scoring";
        let headers = create_l2_headers(
            signer,
            api_creds,
            method.as_str(),
            endpoint,
            Some(order_ids),
        )?;

        let response = self
            .http_client
            .request(method, format!("{}{}", self.base_url, endpoint))
            .headers(
                headers
                    .into_iter()
                    .map(|(k, v)| (HeaderName::from_static(k), v.parse().unwrap()))
                    .collect(),
            )
            .json(order_ids)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<std::collections::HashMap<String, bool>>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    // ============================================================================
    // RFQ (Market Maker) endpoints
    // ============================================================================

    /// Create an RFQ request.
    pub async fn create_rfq_request(
        &self,
        request: &crate::types::RfqCreateRequest,
    ) -> Result<crate::types::RfqCreateRequestResponse> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::POST;
        let endpoint = "/rfq/request";
        let headers =
            create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(request))?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .json(request)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to create RFQ request",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Cancel an RFQ request.
    pub async fn cancel_rfq_request(&self, request_id: &str) -> Result<()> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::DELETE;
        let endpoint = "/rfq/request";
        let body = crate::types::RfqCancelRequest {
            request_id: request_id.to_string(),
        };
        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(&body))?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .json(&body)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to cancel RFQ request",
            ));
        }

        Ok(())
    }

    /// Get RFQ requests (requester).
    pub async fn get_rfq_requests(
        &self,
        params: Option<&crate::types::RfqRequestsParams>,
    ) -> Result<crate::types::RfqListResponse<crate::types::RfqRequestData>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/rfq/data/requests";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = params.cloned().unwrap_or_default().to_query_params();

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get RFQ requests",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Create an RFQ quote.
    pub async fn create_rfq_quote(
        &self,
        quote: &crate::types::RfqCreateQuote,
    ) -> Result<crate::types::RfqCreateQuoteResponse> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::POST;
        let endpoint = "/rfq/quote";
        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(quote))?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .json(quote)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to create RFQ quote",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Cancel an RFQ quote.
    pub async fn cancel_rfq_quote(&self, quote_id: &str) -> Result<()> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::DELETE;
        let endpoint = "/rfq/quote";
        let body = crate::types::RfqCancelQuote {
            quote_id: quote_id.to_string(),
        };
        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(&body))?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .json(&body)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to cancel RFQ quote",
            ));
        }

        Ok(())
    }

    /// Get quotes for the requester.
    pub async fn get_rfq_requester_quotes(
        &self,
        params: Option<&crate::types::RfqQuotesParams>,
    ) -> Result<crate::types::RfqListResponse<crate::types::RfqQuoteData>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/rfq/data/requester/quotes";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = params.cloned().unwrap_or_default().to_query_params();

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get RFQ requester quotes",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get quotes for the quoter.
    pub async fn get_rfq_quoter_quotes(
        &self,
        params: Option<&crate::types::RfqQuotesParams>,
    ) -> Result<crate::types::RfqListResponse<crate::types::RfqQuoteData>> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/rfq/data/quoter/quotes";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let query_params = params.cloned().unwrap_or_default().to_query_params();

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .query(&query_params)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get RFQ quoter quotes",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get best quote for a request.
    pub async fn get_rfq_best_quote(&self, request_id: &str) -> Result<crate::types::RfqQuoteData> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::GET;
        let endpoint = "/rfq/data/best-quote";
        let headers =
            create_l2_headers::<Value>(signer, api_creds, method.as_str(), endpoint, None)?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .query(&[("requestId", request_id)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to get RFQ best quote",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Accept the best quote and post the resulting order.
    pub async fn accept_rfq_quote(
        &self,
        body: &crate::types::RfqOrderExecutionRequest,
    ) -> Result<()> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::POST;
        let endpoint = "/rfq/request/accept";
        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(body))?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .json(body)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to accept RFQ quote",
            ));
        }

        Ok(())
    }

    /// Approve the accepted quote's order (Quoter).
    pub async fn approve_rfq_order(
        &self,
        body: &crate::types::RfqOrderExecutionRequest,
    ) -> Result<crate::types::RfqApproveOrderResponse> {
        let signer = self
            .signer
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self
            .api_creds
            .as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let method = Method::POST;
        let endpoint = "/rfq/quote/approve";
        let headers = create_l2_headers(signer, api_creds, method.as_str(), endpoint, Some(body))?;

        let response = self
            .create_request_with_headers(method, endpoint, headers.into_iter())
            .json(body)
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(
                response.status().as_u16(),
                "Failed to approve RFQ order",
            ));
        }

        response
            .json()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get sampling markets with pagination
    pub async fn get_sampling_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::MarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(format!("{}/sampling-markets", self.base_url))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::MarketsResponse>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get sampling simplified markets with pagination
    pub async fn get_sampling_simplified_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::SimplifiedMarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(format!("{}/sampling-simplified-markets", self.base_url))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::SimplifiedMarketsResponse>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get markets with pagination
    pub async fn get_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::MarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(format!("{}/markets", self.base_url))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::MarketsResponse>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get simplified markets with pagination
    pub async fn get_simplified_markets(
        &self,
        next_cursor: Option<&str>,
    ) -> Result<crate::types::SimplifiedMarketsResponse> {
        let next_cursor = next_cursor.unwrap_or("MA=="); // INITIAL_CURSOR

        let response = self
            .http_client
            .get(format!("{}/simplified-markets", self.base_url))
            .query(&[("next_cursor", next_cursor)])
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::SimplifiedMarketsResponse>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get single market by condition ID
    pub async fn get_market(&self, condition_id: &str) -> Result<crate::types::Market> {
        let response = self
            .http_client
            .get(format!("{}/markets/{}", self.base_url, condition_id))
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<crate::types::Market>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }

    /// Get market trades events
    pub async fn get_market_trades_events(&self, condition_id: &str) -> Result<Value> {
        let response = self
            .http_client
            .get(format!(
                "{}/live-activity/events/{}",
                self.base_url, condition_id
            ))
            .send()
            .await
            .map_err(|e| PolyfillError::network(format!("Request failed: {}", e), e))?;

        response
            .json::<Value>()
            .await
            .map_err(|e| PolyfillError::parse(format!("Failed to parse response: {}", e), None))
    }
}

// Re-export types from the canonical location in types.rs
pub use crate::types::{
    ExtraOrderArgs, Market, MarketOrderArgs, MarketsResponse, MidpointResponse, NegRiskResponse,
    OrderBookSummary, OrderSummary, PriceResponse, PricesHistoryInterval, PricesHistoryResponse,
    Rewards, SpreadResponse, TickSizeResponse, Token,
};

// Compatibility types that need to stay in client.rs
#[derive(Debug, Default)]
pub struct CreateOrderOptions {
    pub tick_size: Option<Decimal>,
    pub neg_risk: Option<bool>,
}

// Re-export for compatibility
pub type PolyfillClient = ClobClient;

#[cfg(test)]
mod tests {
    use super::{ClobClient, OrderArgs as ClientOrderArgs};
    use crate::types::{
        PricesHistoryInterval, RfqCreateQuote, RfqCreateRequest, RfqOrderExecutionRequest,
        RfqQuotesParams, RfqRequestsParams, Side,
    };
    use crate::{ApiCredentials, PolyfillError};
    use mockito::{Matcher, Server};
    use rust_decimal::Decimal;
    use serde_json::json;
    use std::str::FromStr;
    use tokio;

    fn create_test_client(base_url: &str) -> ClobClient {
        ClobClient::new(base_url)
    }

    fn create_test_client_with_auth(base_url: &str) -> ClobClient {
        ClobClient::with_l1_headers(
            base_url,
            "0x1234567890123456789012345678901234567890123456789012345678901234",
            137,
        )
    }

    fn create_test_client_with_l2_auth(base_url: &str) -> ClobClient {
        let api_creds = ApiCredentials {
            api_key: "test_key".to_string(),
            // URL-safe base64 so HMAC header generation succeeds.
            secret: "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1".to_string(),
            passphrase: "test_passphrase".to_string(),
        };

        ClobClient::with_l2_headers(
            base_url,
            "0x1234567890123456789012345678901234567890123456789012345678901234",
            137,
            api_creds,
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_client_creation() {
        let client = create_test_client("https://test.example.com");
        assert_eq!(client.base_url, "https://test.example.com");
        assert!(client.signer.is_none());
        assert!(client.api_creds.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_client_with_l1_headers() {
        let client = create_test_client_with_auth("https://test.example.com");
        assert_eq!(client.base_url, "https://test.example.com");
        assert!(client.signer.is_some());
        assert_eq!(client.chain_id, 137);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_client_with_l2_headers() {
        let api_creds = ApiCredentials {
            api_key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            passphrase: "test_passphrase".to_string(),
        };

        let client = ClobClient::with_l2_headers(
            "https://test.example.com",
            "0x1234567890123456789012345678901234567890123456789012345678901234",
            137,
            api_creds.clone(),
        );

        assert_eq!(client.base_url, "https://test.example.com");
        assert!(client.signer.is_some());
        assert!(client.api_creds.is_some());
        assert_eq!(client.chain_id, 137);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_api_creds() {
        let mut client = create_test_client("https://test.example.com");
        assert!(client.api_creds.is_none());

        let api_creds = ApiCredentials {
            api_key: "test_key".to_string(),
            secret: "test_secret".to_string(),
            passphrase: "test_passphrase".to_string(),
        };

        client.set_api_creds(api_creds.clone());
        assert!(client.api_creds.is_some());
        assert_eq!(client.api_creds.unwrap().api_key, "test_key");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_sampling_markets_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "limit": 10,
            "count": 2, 
            "next_cursor": null,
            "data": [
                {
                    "condition_id": "0x123",
                    "tokens": [
                        {"token_id": "0x456", "outcome": "Yes", "price": 0.5, "winner": false},
                        {"token_id": "0x789", "outcome": "No", "price": 0.5, "winner": false}
                    ],
                    "rewards": {
                        "rates": null,
                        "min_size": 1.0,
                        "max_spread": 0.1,
                        "event_start_date": null,
                        "event_end_date": null,
                        "in_game_multiplier": null,
                        "reward_epoch": null
                    },
                    "min_incentive_size": null,
                    "max_incentive_spread": null,
                    "active": true,
                    "closed": false,
                    "question_id": "0x123",
                    "minimum_order_size": 1.0,
                    "minimum_tick_size": 0.01,
                    "description": "Test market",
                    "category": "test",
                    "end_date_iso": null,
                    "game_start_time": null,
                    "question": "Will this test pass?",
                    "market_slug": "test-market",
                    "seconds_delay": 0,
                    "icon": "",
                    "fpmm": ""
                }
            ]
        }"#;

        let mock = server
            .mock("GET", "/sampling-markets")
            .match_query(Matcher::UrlEncoded("next_cursor".into(), "MA==".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_sampling_markets(None).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let markets = result.unwrap();
        assert_eq!(markets.data.len(), 1);
        assert_eq!(markets.data[0].question, "Will this test pass?");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_sampling_markets_with_cursor() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "limit": 5,
            "count": 0,
            "next_cursor": null,
            "data": []
        }"#;

        let mock = server
            .mock("GET", "/sampling-markets")
            .match_query(Matcher::AllOf(vec![Matcher::UrlEncoded(
                "next_cursor".into(),
                "test_cursor".into(),
            )]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_sampling_markets(Some("test_cursor")).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let markets = result.unwrap();
        assert_eq!(markets.data.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_order_book_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "market": "0x123",
            "asset_id": "0x123",
            "hash": "0xabc123",
            "timestamp": "1234567890",
            "bids": [
                {"price": "0.75", "size": "100.0"}
            ],
            "asks": [
                {"price": "0.76", "size": "50.0"}
            ],
            "min_order_size": "1",
            "neg_risk": false,
            "tick_size": "0.01",
            "last_trade_price": "0.755"
        }"#;

        let mock = server
            .mock("GET", "/book")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_order_book("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let book = result.unwrap();
        assert_eq!(book.market, "0x123");
        assert_eq!(book.bids.len(), 1);
        assert_eq!(book.asks.len(), 1);
        assert_eq!(book.min_order_size, Decimal::from_str("1").unwrap());
        assert!(!book.neg_risk);
        assert_eq!(book.tick_size, Decimal::from_str("0.01").unwrap());
        assert_eq!(
            book.last_trade_price,
            Some(Decimal::from_str("0.755").unwrap())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_midpoint_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "mid": "0.755"
        }"#;

        let mock = server
            .mock("GET", "/midpoint")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_midpoint("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.mid, Decimal::from_str("0.755").unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_spread_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "spread": "0.01"
        }"#;

        let mock = server
            .mock("GET", "/spread")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_spread("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.spread, Decimal::from_str("0.01").unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_price_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "price": "0.76"
        }"#;

        let mock = server
            .mock("GET", "/price")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("token_id".into(), "0x123".into()),
                Matcher::UrlEncoded("side".into(), "BUY".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_price("0x123", Side::BUY).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.price, Decimal::from_str("0.76").unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_prices_history_interval_rejects_hex_condition_id() {
        let client = create_test_client("https://test.example.com");
        let result = client
            .get_prices_history_interval("0xdeadbeef", PricesHistoryInterval::OneDay, None)
            .await;
        assert!(matches!(result, Err(PolyfillError::Validation { .. })));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_prices_history_interval_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{"history":[{"t":1}]}"#;

        let mock = server
            .mock("GET", "/prices-history")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("market".into(), "12345".into()),
                Matcher::UrlEncoded("interval".into(), "1d".into()),
                Matcher::UrlEncoded("fidelity".into(), "5".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let response = client
            .get_prices_history_interval("12345", PricesHistoryInterval::OneDay, Some(5))
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(response.history.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_tick_size_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "minimum_tick_size": "0.01"
        }"#;

        let mock = server
            .mock("GET", "/tick-size")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_tick_size("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let tick_size = result.unwrap();
        assert_eq!(tick_size, Decimal::from_str("0.01").unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_neg_risk_success() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "neg_risk": false
        }"#;

        let mock = server
            .mock("GET", "/neg-risk")
            .match_query(Matcher::UrlEncoded("token_id".into(), "0x123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_neg_risk("0x123").await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let neg_risk = result.unwrap();
        assert!(!neg_risk);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_api_error_handling() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/book")
            .match_query(Matcher::UrlEncoded(
                "token_id".into(),
                "invalid_token".into(),
            ))
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Market not found"}"#)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_order_book("invalid_token").await;

        mock.assert_async().await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        // The error should be either Network or Api error
        assert!(
            matches!(error, PolyfillError::Network { .. })
                || matches!(error, PolyfillError::Api { .. })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_network_error_handling() {
        // Test with invalid URL to simulate network error
        let client = create_test_client("http://invalid-host-that-does-not-exist.com");
        let result = client.get_order_book("0x123").await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(matches!(error, PolyfillError::Network { .. }));
    }

    #[test]
    fn test_client_url_validation() {
        let client = create_test_client("https://test.example.com");
        assert_eq!(client.base_url, "https://test.example.com");

        let client2 = create_test_client("http://localhost:8080");
        assert_eq!(client2.base_url, "http://localhost:8080");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_midpoints_batch() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "0x123": "0.755",
            "0x456": "0.623"
        }"#;

        let mock = server
            .mock("POST", "/midpoints")
            .with_header("content-type", "application/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let token_ids = vec!["0x123".to_string(), "0x456".to_string()];
        let result = client.get_midpoints(&token_ids).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let midpoints = result.unwrap();
        assert_eq!(midpoints.len(), 2);
        assert_eq!(
            midpoints.get("0x123").unwrap(),
            &Decimal::from_str("0.755").unwrap()
        );
        assert_eq!(
            midpoints.get("0x456").unwrap(),
            &Decimal::from_str("0.623").unwrap()
        );
    }

    #[test]
    fn test_client_configuration() {
        let client = create_test_client("https://test.example.com");

        // Test initial state
        assert!(client.signer.is_none());
        assert!(client.api_creds.is_none());

        // Test with auth
        let auth_client = create_test_client_with_auth("https://test.example.com");
        assert!(auth_client.signer.is_some());
        assert_eq!(auth_client.chain_id, 137);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_ok() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{"status": "ok"}"#;

        let mock = server
            .mock("GET", "/ok")
            .with_header("content-type", "application/json")
            .with_status(200)
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_ok().await;

        mock.assert_async().await;
        assert!(result);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_prices_batch() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "0x123": {
                "BUY": "0.755",
                "SELL": "0.745"
            },
            "0x456": {
                "BUY": "0.623",
                "SELL": "0.613"
            }
        }"#;

        let mock = server
            .mock("POST", "/prices")
            .with_header("content-type", "application/json")
            .with_status(200)
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let book_params = vec![
            crate::types::BookParams {
                token_id: "0x123".to_string(),
                side: Side::BUY,
            },
            crate::types::BookParams {
                token_id: "0x456".to_string(),
                side: Side::SELL,
            },
        ];
        let result = client.get_prices(&book_params).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let prices = result.unwrap();
        assert_eq!(prices.len(), 2);
        assert!(prices.contains_key("0x123"));
        assert!(prices.contains_key("0x456"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_server_time() {
        let mut server = Server::new_async().await;
        let mock_response = "1234567890"; // Plain text response

        let mock = server
            .mock("GET", "/time")
            .with_status(200)
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let result = client.get_server_time().await;

        mock.assert_async().await;
        assert!(result.is_ok());
        let timestamp = result.unwrap();
        assert_eq!(timestamp, 1234567890);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_or_derive_api_key() {
        let mut server = Server::new_async().await;
        let mock_response = r#"{
            "apiKey": "test-api-key-123",
            "secret": "test-secret-456",
            "passphrase": "test-passphrase"
        }"#;

        // Mock both create and derive endpoints since the method tries both
        let create_mock = server
            .mock("POST", "/auth/api-key")
            .with_header("content-type", "application/json")
            .with_status(200)
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client_with_auth(&server.url());
        let result = client.create_or_derive_api_key(None).await;

        create_mock.assert_async().await;
        assert!(result.is_ok());
        let api_creds = result.unwrap();
        assert_eq!(api_creds.api_key, "test-api-key-123");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_or_derive_api_key_falls_back_on_api_error() {
        let mut server = Server::new_async().await;

        // Create fails with a status error -> should fall back to derive.
        let create_mock = server
            .mock("POST", "/auth/api-key")
            .with_status(400)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error":"key exists"}"#)
            .create_async()
            .await;

        let derive_mock = server
            .mock("GET", "/auth/derive-api-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"apiKey":"derived-api-key","secret":"derived-secret","passphrase":"derived-pass"}"#,
            )
            .create_async()
            .await;

        let client = create_test_client_with_auth(&server.url());
        let result = client.create_or_derive_api_key(None).await;

        create_mock.assert_async().await;
        derive_mock.assert_async().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().api_key, "derived-api-key");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_or_derive_api_key_does_not_fallback_on_non_api_error() {
        let mut server = Server::new_async().await;

        // Create returns 200 but with invalid JSON -> not an API status error.
        let create_mock = server
            .mock("POST", "/auth/api-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not-json")
            .create_async()
            .await;

        // If we incorrectly fall back, this would be called.
        let derive_mock = server
            .mock("GET", "/auth/derive-api-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"apiKey":"derived-api-key","secret":"derived-secret","passphrase":"derived-pass"}"#,
            )
            .expect(0)
            .create_async()
            .await;

        let client = create_test_client_with_auth(&server.url());
        let result = client.create_or_derive_api_key(None).await;

        create_mock.assert_async().await;
        derive_mock.assert_async().await;
        assert!(result.is_err());
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_order_books_batch() {
        let mut server = Server::new_async().await;
        let mock_response = r#"[
            {
                "market": "0x123",
                "asset_id": "0x123",
                "hash": "test-hash",
                "timestamp": "1234567890",
                "bids": [{"price": "0.75", "size": "100.0"}],
                "asks": [{"price": "0.76", "size": "50.0"}],
                "min_order_size": "1",
                "neg_risk": false,
                "tick_size": "0.01",
                "last_trade_price": null
            }
        ]"#;

        let mock = server
            .mock("POST", "/books")
            .with_header("content-type", "application/json")
            .with_status(200)
            .with_body(mock_response)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let token_ids = vec!["0x123".to_string()];
        let result = client.get_order_books(&token_ids).await;

        mock.assert_async().await;
        if let Err(e) = &result {
            println!("Error: {:?}", e);
        }
        assert!(result.is_ok());
        let books = result.unwrap();
        assert_eq!(books.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_order_args_creation() {
        // Test OrderArgs creation and default values
        let order_args = ClientOrderArgs::new(
            "0x123",
            Decimal::from_str("0.75").unwrap(),
            Decimal::from_str("100.0").unwrap(),
            Side::BUY,
        );

        assert_eq!(order_args.token_id, "0x123");
        assert_eq!(order_args.price, Decimal::from_str("0.75").unwrap());
        assert_eq!(order_args.size, Decimal::from_str("100.0").unwrap());
        assert_eq!(order_args.side, Side::BUY);

        // Test default
        let default_args = ClientOrderArgs::default();
        assert_eq!(default_args.token_id, "");
        assert_eq!(default_args.price, Decimal::ZERO);
        assert_eq!(default_args.size, Decimal::ZERO);
        assert_eq!(default_args.side, Side::BUY);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_fee_rate_bps_success() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/fee-rate")
            .match_query(Matcher::UrlEncoded("token_id".into(), "123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"fee_rate_bps":1000}"#)
            .create_async()
            .await;

        let client = create_test_client(&server.url());
        let rate = client.get_fee_rate_bps("123").await.unwrap();

        mock.assert_async().await;
        assert_eq!(rate, 1000);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_rfq_endpoints_happy_path() {
        let mut server = Server::new_async().await;

        // create_rfq_request
        let create_request = RfqCreateRequest {
            asset_in: "some_asset_in".to_string(),
            asset_out: "some_asset_out".to_string(),
            amount_in: "100".to_string(),
            amount_out: "200".to_string(),
            user_type: 0,
        };
        let create_request_mock = server
            .mock("POST", "/rfq/request")
            .match_body(Matcher::JsonString(
                json!({
                    "assetIn": "some_asset_in",
                    "assetOut": "some_asset_out",
                    "amountIn": "100",
                    "amountOut": "200",
                    "userType": 0
                })
                .to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"requestId":"req123","expiry":1744936318}"#)
            .create_async()
            .await;

        // cancel_rfq_request
        let cancel_request_mock = server
            .mock("DELETE", "/rfq/request")
            .match_body(Matcher::JsonString(r#"{"requestId":"req123"}"#.to_string()))
            .with_status(200)
            .with_body("OK")
            .create_async()
            .await;

        // get_rfq_requests
        let rfq_requests_mock = server
            .mock("GET", "/rfq/data/requests")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("offset".into(), "MA==".into()),
                Matcher::UrlEncoded("limit".into(), "10".into()),
                Matcher::UrlEncoded("state".into(), "active".into()),
                Matcher::UrlEncoded("requestIds[]".into(), "req123".into()),
                Matcher::UrlEncoded("markets[]".into(), "some_market".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [{
                        "requestId": "req123",
                        "userAddress": "0xabc",
                        "proxyAddress": "0xdef",
                        "condition": "some_condition_id",
                        "token": "some_token_id",
                        "complement": "some_complement",
                        "side": "BUY",
                        "sizeIn": 100,
                        "sizeOut": 200,
                        "price": 0.5,
                        "state": "active",
                        "expiry": 1744936318
                    }],
                    "next_cursor": "MA==",
                    "limit": 10,
                    "count": 1
                }"#,
            )
            .create_async()
            .await;

        // create_rfq_quote
        let create_quote = RfqCreateQuote {
            request_id: "req123".to_string(),
            asset_in: "some_asset_in".to_string(),
            asset_out: "some_asset_out".to_string(),
            amount_in: "100".to_string(),
            amount_out: "200".to_string(),
            user_type: 0,
        };
        let create_quote_mock = server
            .mock("POST", "/rfq/quote")
            .match_body(Matcher::JsonString(
                json!({
                    "requestId": "req123",
                    "assetIn": "some_asset_in",
                    "assetOut": "some_asset_out",
                    "amountIn": "100",
                    "amountOut": "200",
                    "userType": 0
                })
                .to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"quoteId":"q123"}"#)
            .create_async()
            .await;

        // cancel_rfq_quote
        let cancel_quote_mock = server
            .mock("DELETE", "/rfq/quote")
            .match_body(Matcher::JsonString(r#"{"quoteId":"q123"}"#.to_string()))
            .with_status(200)
            .with_body("OK")
            .create_async()
            .await;

        // get_rfq_requester_quotes
        let requester_quotes_mock = server
            .mock("GET", "/rfq/data/requester/quotes")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("offset".into(), "MA==".into()),
                Matcher::UrlEncoded("limit".into(), "10".into()),
                Matcher::UrlEncoded("state".into(), "active".into()),
                Matcher::UrlEncoded("quoteIds[]".into(), "q123".into()),
                Matcher::UrlEncoded("requestIds[]".into(), "req123".into()),
            ]))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [{
                        "quoteId": "q123",
                        "requestId": "req123",
                        "userAddress": "0xabc",
                        "proxyAddress": "0xdef",
                        "condition": "some_condition_id",
                        "token": "some_token_id",
                        "complement": "some_complement",
                        "side": "BUY",
                        "sizeIn": 100,
                        "sizeOut": 200,
                        "price": 0.5,
                        "matchType": "matched",
                        "state": "active"
                    }],
                    "next_cursor": "MA==",
                    "limit": 10,
                    "count": 1
                }"#,
            )
            .create_async()
            .await;

        // get_rfq_quoter_quotes
        let quoter_quotes_mock = server
            .mock("GET", "/rfq/data/quoter/quotes")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "data": [],
                    "next_cursor": "MA==",
                    "limit": 10,
                    "count": 0
                }"#,
            )
            .create_async()
            .await;

        // get_rfq_best_quote
        let best_quote_mock = server
            .mock("GET", "/rfq/data/best-quote")
            .match_query(Matcher::UrlEncoded("requestId".into(), "req123".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "quoteId": "q123",
                    "requestId": "req123",
                    "userAddress": "0xabc",
                    "proxyAddress": "0xdef",
                    "condition": "some_condition_id",
                    "token": "some_token_id",
                    "complement": "some_complement",
                    "side": "BUY",
                    "sizeIn": 100,
                    "sizeOut": 200,
                    "price": 0.5,
                    "matchType": "matched",
                    "state": "active"
                }"#,
            )
            .create_async()
            .await;

        // accept_rfq_quote
        let exec = RfqOrderExecutionRequest {
            request_id: "req123".to_string(),
            quote_id: "q123".to_string(),
            maker: "0xmaker".to_string(),
            signer: "0xsigner".to_string(),
            taker: "0xtaker".to_string(),
            expiration: 1_740_000_000,
            nonce: "123".to_string(),
            fee_rate_bps: "1000".to_string(),
            side: "BUY".to_string(),
            token_id: "123".to_string(),
            maker_amount: "100".to_string(),
            taker_amount: "200".to_string(),
            signature_type: 2,
            signature: "0xsig".to_string(),
            salt: 42,
            owner: "owner".to_string(),
        };

        let accept_mock = server
            .mock("POST", "/rfq/request/accept")
            .match_body(Matcher::JsonString(
                json!({
                    "requestId": "req123",
                    "quoteId": "q123",
                    "maker": "0xmaker",
                    "signer": "0xsigner",
                    "taker": "0xtaker",
                    "expiration": 1740000000,
                    "nonce": "123",
                    "feeRateBps": "1000",
                    "side": "BUY",
                    "tokenId": "123",
                    "makerAmount": "100",
                    "takerAmount": "200",
                    "signatureType": 2,
                    "signature": "0xsig",
                    "salt": 42,
                    "owner": "owner"
                })
                .to_string(),
            ))
            .with_status(200)
            .with_body("OK")
            .create_async()
            .await;

        // approve_rfq_order
        let approve_mock = server
            .mock("POST", "/rfq/quote/approve")
            .match_body(Matcher::JsonString(
                json!({
                    "requestId": "req123",
                    "quoteId": "q123",
                    "maker": "0xmaker",
                    "signer": "0xsigner",
                    "taker": "0xtaker",
                    "expiration": 1740000000,
                    "nonce": "123",
                    "feeRateBps": "1000",
                    "side": "BUY",
                    "tokenId": "123",
                    "makerAmount": "100",
                    "takerAmount": "200",
                    "signatureType": 2,
                    "signature": "0xsig",
                    "salt": 42,
                    "owner": "owner"
                })
                .to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tradeIds":["t1","t2"]}"#)
            .create_async()
            .await;

        let client = create_test_client_with_l2_auth(&server.url());

        let created = client.create_rfq_request(&create_request).await.unwrap();
        assert_eq!(created.request_id, "req123");
        assert_eq!(created.expiry, 1_744_936_318);
        create_request_mock.assert_async().await;

        client.cancel_rfq_request("req123").await.unwrap();
        cancel_request_mock.assert_async().await;

        let params = RfqRequestsParams {
            offset: Some("MA==".to_string()),
            limit: Some(10),
            state: Some("active".to_string()),
            request_ids: vec!["req123".to_string()],
            markets: vec!["some_market".to_string()],
            ..Default::default()
        };
        let requests = client.get_rfq_requests(Some(&params)).await.unwrap();
        assert_eq!(requests.data.len(), 1);
        assert_eq!(requests.data[0].request_id, "req123");
        rfq_requests_mock.assert_async().await;

        let quote = client.create_rfq_quote(&create_quote).await.unwrap();
        assert_eq!(quote.quote_id, "q123");
        create_quote_mock.assert_async().await;

        client.cancel_rfq_quote("q123").await.unwrap();
        cancel_quote_mock.assert_async().await;

        let quote_params = RfqQuotesParams {
            offset: Some("MA==".to_string()),
            limit: Some(10),
            state: Some("active".to_string()),
            quote_ids: vec!["q123".to_string()],
            request_ids: vec!["req123".to_string()],
            ..Default::default()
        };

        let requester_quotes = client
            .get_rfq_requester_quotes(Some(&quote_params))
            .await
            .unwrap();
        assert_eq!(requester_quotes.data.len(), 1);
        requester_quotes_mock.assert_async().await;

        let quoter_quotes = client.get_rfq_quoter_quotes(None).await.unwrap();
        assert_eq!(quoter_quotes.data.len(), 0);
        quoter_quotes_mock.assert_async().await;

        let best = client.get_rfq_best_quote("req123").await.unwrap();
        assert_eq!(best.quote_id, "q123");
        best_quote_mock.assert_async().await;

        client.accept_rfq_quote(&exec).await.unwrap();
        accept_mock.assert_async().await;

        let approved = client.approve_rfq_order(&exec).await.unwrap();
        assert_eq!(approved.trade_ids, vec!["t1".to_string(), "t2".to_string()]);
        approve_mock.assert_async().await;
    }
}
