//! High-performance Rust client for Polymarket
//! 
//! This module provides a production-ready client for interacting with
//! Polymarket, optimized for high-frequency trading environments.

use crate::auth::{create_l1_headers, create_l2_headers};
use crate::errors::{PolyfillError, Result};
use crate::types::{OrderOptions, PostOrder, SignedOrderRequest};
use reqwest::Client;
use serde_json::Value;
use std::str::FromStr;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use alloy_primitives::U256;
use alloy_signer_local::PrivateKeySigner;
use reqwest::{Method, RequestBuilder};
use reqwest::header::HeaderName;

// Re-export types for compatibility
pub use crate::types::{
    ApiCredentials as ApiCreds, Side, OrderType,
};

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
    http_client: Client,
    base_url: String,
    chain_id: u64,
    signer: Option<PrivateKeySigner>,
    api_creds: Option<ApiCreds>,
    order_builder: Option<crate::orders::OrderBuilder>,
}

impl ClobClient {
    /// Create a new client
    pub fn new(host: &str) -> Self {
        Self {
            http_client: Client::new(),
            base_url: host.to_string(),
            chain_id: 137, // Default to Polygon
            signer: None,
            api_creds: None,
            order_builder: None,
        }
    }

    /// Create a client with L1 headers (for authentication)
    pub fn with_l1_headers(host: &str, private_key: &str, chain_id: u64) -> Self {
        let signer = private_key.parse::<PrivateKeySigner>()
            .expect("Invalid private key");
        
        let order_builder = crate::orders::OrderBuilder::new(signer.clone(), None, None);
        
        Self {
            http_client: Client::new(),
            base_url: host.to_string(),
            chain_id,
            signer: Some(signer),
            api_creds: None,
            order_builder: Some(order_builder),
        }
    }

    /// Set API credentials
    pub fn set_api_creds(&mut self, api_creds: ApiCreds) {
        self.api_creds = Some(api_creds);
    }

    /// Test basic connectivity
    pub async fn get_ok(&self) -> bool {
        match self.http_client.get(&format!("{}/ok", self.base_url)).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }

    /// Get server time
    pub async fn get_server_time(&self) -> Result<u64> {
        let response = self.http_client
            .get(&format!("{}/time", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get server time"));
        }

        let time_text = response.text().await?;
        let timestamp = time_text.trim()
            .parse::<u64>()
            .map_err(|e| PolyfillError::parse(format!("Invalid timestamp format: {}", e), None))?;

        Ok(timestamp)
    }

    /// Get sampling markets
    pub async fn get_sampling_markets(&self, _limit: Option<u32>) -> Result<MarketsResponse> {
        let response = self.http_client
            .get(&format!("{}/sampling-markets", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get sampling markets"));
        }

        let markets_response: MarketsResponse = response.json().await?;
        Ok(markets_response)
    }

    /// Get order book for a token
    pub async fn get_order_book(&self, token_id: &str) -> Result<OrderBookSummary> {
        let response = self.http_client
            .get(&format!("{}/book", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get order book"));
        }

        let order_book: OrderBookSummary = response.json().await?;
        Ok(order_book)
    }

    /// Get midpoint for a token
    pub async fn get_midpoint(&self, token_id: &str) -> Result<MidpointResponse> {
        let response = self.http_client
            .get(&format!("{}/midpoint", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get midpoint"));
        }

        let midpoint: MidpointResponse = response.json().await?;
        Ok(midpoint)
    }

    /// Get spread for a token
    pub async fn get_spread(&self, token_id: &str) -> Result<SpreadResponse> {
        let response = self.http_client
            .get(&format!("{}/spread", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get spread"));
        }

        let spread: SpreadResponse = response.json().await?;
        Ok(spread)
    }

    /// Get price for a token and side
    pub async fn get_price(&self, token_id: &str, side: Side) -> Result<PriceResponse> {
        let response = self.http_client
            .get(&format!("{}/price", self.base_url))
            .query(&[
                ("token_id", token_id),
                ("side", side.as_str()),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get price"));
        }

        let price: PriceResponse = response.json().await?;
        Ok(price)
    }

    /// Get tick size for a token
    pub async fn get_tick_size(&self, token_id: &str) -> Result<Decimal> {
        let response = self.http_client
            .get(&format!("{}/tick-size", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get tick size"));
        }

        let tick_size_response: Value = response.json().await?;
        let tick_size = tick_size_response["minimum_tick_size"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .or_else(|| tick_size_response["minimum_tick_size"].as_f64().map(|f| Decimal::from_f64(f).unwrap_or(Decimal::ZERO)))
            .ok_or_else(|| PolyfillError::parse("Invalid tick size format", None))?;

        Ok(tick_size)
    }

    /// Create a new API key
    pub async fn create_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        
        let headers = create_l1_headers(signer, nonce)?;
        let req = self.create_request_with_headers(Method::POST, "/auth/api-key", headers.into_iter());
        
        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to create API key"));
        }
        
        Ok(response.json::<ApiCreds>().await?)
    }

    /// Derive an existing API key
    pub async fn derive_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        
        let headers = create_l1_headers(signer, nonce)?;
        let req = self.create_request_with_headers(Method::GET, "/auth/derive-api-key", headers.into_iter());
        
        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to derive API key"));
        }
        
        Ok(response.json::<ApiCreds>().await?)
    }

    /// Create or derive API key (try create first, fallback to derive)
    pub async fn create_or_derive_api_key(&self, nonce: Option<U256>) -> Result<ApiCreds> {
        match self.create_api_key(nonce).await {
            Ok(creds) => Ok(creds),
            Err(_) => self.derive_api_key(nonce).await,
        }
    }

    /// Helper to create request with headers
    fn create_request_with_headers(
        &self,
        method: Method,
        endpoint: &str,
        headers: impl Iterator<Item = (&'static str, String)>,
    ) -> RequestBuilder {
        let req = self.http_client.request(method, format!("{}{}", &self.base_url, endpoint));
        headers.fold(req, |r, (k, v)| r.header(HeaderName::from_static(k), v))
    }

    /// Get neg risk for a token
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        let response = self.http_client
            .get(&format!("{}/neg-risk", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get neg risk"));
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
            }
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
        let order_builder = self.order_builder.as_ref()
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
            return Err(PolyfillError::validation("Price is not in range of tick_size"));
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
        let order_builder = self.order_builder.as_ref()
            .ok_or_else(|| PolyfillError::auth("Order builder not initialized"))?;

        // Convert OrderSummary to BookLevel
        let levels: Vec<crate::types::BookLevel> = match side {
            Side::BUY => book.asks.into_iter().map(|s| crate::types::BookLevel {
                price: s.price,
                size: s.size,
            }).collect(),
            Side::SELL => book.bids.into_iter().map(|s| crate::types::BookLevel {
                price: s.price,
                size: s.size,
            }).collect(),
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
        let order_builder = self.order_builder.as_ref()
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
            return Err(PolyfillError::validation("Price is not in range of tick_size"));
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
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let body = PostOrder::new(order, api_creds.api_key.clone(), order_type);

        let headers = create_l2_headers(signer, api_creds, "POST", "/order", Some(&body))?;
        let req = self.create_request_with_headers(Method::POST, "/order", headers.into_iter());

        let response = req.json(&body).send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to post order"));
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
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let body = std::collections::HashMap::from([("orderID", order_id)]);

        let headers = create_l2_headers(signer, api_creds, "DELETE", "/order", Some(&body))?;
        let req = self.create_request_with_headers(Method::DELETE, "/order", headers.into_iter());

        let response = req.json(&body).send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to cancel order"));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Cancel multiple orders
    pub async fn cancel_orders(&self, order_ids: &[String]) -> Result<Value> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers(signer, api_creds, "DELETE", "/orders", Some(order_ids))?;
        let req = self.create_request_with_headers(Method::DELETE, "/orders", headers.into_iter());

        let response = req.json(order_ids).send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to cancel orders"));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Cancel all orders
    pub async fn cancel_all(&self) -> Result<Value> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers::<Value>(signer, api_creds, "DELETE", "/cancel-all", None)?;
        let req = self.create_request_with_headers(Method::DELETE, "/cancel-all", headers.into_iter());

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to cancel all orders"));
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
    pub async fn get_orders(&self, params: Option<crate::types::OpenOrderParams>) -> Result<Vec<crate::types::OpenOrder>> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers::<Value>(signer, api_creds, "GET", "/orders", None)?;
        let mut req = self.create_request_with_headers(Method::GET, "/orders", headers.into_iter());

        // Add query parameters if provided
        if let Some(params) = params {
            let query_params = params.to_query_params();
            req = req.query(&query_params);
        }

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get orders"));
        }

        let orders: Vec<crate::types::OpenOrder> = response.json().await?;
        Ok(orders)
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
    pub async fn get_trades(&self, params: Option<crate::types::TradeParams>) -> Result<Vec<crate::types::FillEvent>> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers::<Value>(signer, api_creds, "GET", "/trades", None)?;
        let mut req = self.create_request_with_headers(Method::GET, "/trades", headers.into_iter());

        // Add query parameters if provided
        if let Some(params) = params {
            let query_params = params.to_query_params();
            req = req.query(&query_params);
        }

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get trades"));
        }

        let trades: Vec<crate::types::FillEvent> = response.json().await?;
        Ok(trades)
    }

    /// Get balance and allowance information for all assets
    /// 
    /// This returns the current balance and allowance for each asset in your account.
    /// Balance is how much you own, allowance is how much the exchange can spend on your behalf.
    /// 
    /// You need both balance and allowance to place orders - the exchange needs permission
    /// to move your tokens when orders are filled.
    pub async fn balance_allowance(&self) -> Result<Vec<crate::types::BalanceAllowance>> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers::<Value>(signer, api_creds, "GET", "/balance-allowance", None)?;
        let req = self.create_request_with_headers(Method::GET, "/balance-allowance", headers.into_iter());

        let response = req.send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get balance allowance"));
        }

        let balances: Vec<crate::types::BalanceAllowance> = response.json().await?;
        Ok(balances)
    }

    /// Set up notifications for order fills and other events
    /// 
    /// This configures push notifications so you get alerted when:
    /// - Your orders get filled
    /// - Your orders get cancelled
    /// - Market conditions change significantly
    /// 
    /// The signature proves you own the account and want to receive notifications.
    pub async fn notifications(&self, params: crate::types::NotificationParams) -> Result<Value> {
        let signer = self.signer.as_ref()
            .ok_or_else(|| PolyfillError::auth("Signer not set"))?;
        let api_creds = self.api_creds.as_ref()
            .ok_or_else(|| PolyfillError::auth("API credentials not set"))?;

        let headers = create_l2_headers(signer, api_creds, "POST", "/notifications", Some(&params))?;
        let req = self.create_request_with_headers(Method::POST, "/notifications", headers.into_iter());

        let response = req.json(&params).send().await?;
        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to set up notifications"));
        }

        Ok(response.json::<Value>().await?)
    }

    /// Get midpoints for multiple tokens in a single request
    /// 
    /// This is much more efficient than calling get_midpoint() multiple times.
    /// Instead of N round trips, you make just 1 request and get all the midpoints back.
    /// 
    /// Midpoints are returned as a HashMap where the key is the token_id and the value
    /// is the midpoint price (or None if there's no valid midpoint).
    pub async fn get_midpoints(&self, token_ids: Vec<String>) -> Result<crate::types::BatchMidpointResponse> {
        let request = crate::types::BatchMidpointRequest { token_ids };
        
        let response = self.http_client
            .post(&format!("{}/midpoints", self.base_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get batch midpoints"));
        }

        let midpoints: crate::types::BatchMidpointResponse = response.json().await?;
        Ok(midpoints)
    }

    /// Get bid/ask/mid prices for multiple tokens in a single request
    /// 
    /// This gives you the full price picture for multiple tokens at once.
    /// Much more efficient than individual calls, especially when you're tracking
    /// a portfolio or comparing multiple markets.
    /// 
    /// Returns bid (best buy price), ask (best sell price), and mid (average) for each token.
    pub async fn get_prices(&self, token_ids: Vec<String>) -> Result<crate::types::BatchPriceResponse> {
        let request = crate::types::BatchPriceRequest { token_ids };
        
        let response = self.http_client
            .post(&format!("{}/prices", self.base_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(PolyfillError::api(response.status().as_u16(), "Failed to get batch prices"));
        }

        let prices: crate::types::BatchPriceResponse = response.json().await?;
        Ok(prices)
    }
}

// Response types for API calls
#[derive(Debug, serde::Deserialize)]
pub struct MarketsResponse {
    pub limit: Decimal,
    pub count: Decimal,
    pub next_cursor: Option<String>,
    pub data: Vec<Market>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Market {
    pub condition_id: String,
    pub tokens: [Token; 2],
    pub rewards: Rewards,
    pub min_incentive_size: Option<String>,
    pub max_incentive_spread: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub question_id: String,
    pub minimum_order_size: Decimal,
    pub minimum_tick_size: Decimal,
    pub description: String,
    pub category: Option<String>,
    pub end_date_iso: Option<String>,
    pub game_start_time: Option<String>,
    pub question: String,
    pub market_slug: String,
    pub seconds_delay: Decimal,
    pub icon: String,
    pub fpmm: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Token {
    pub token_id: String,
    pub outcome: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct Rewards {
    pub rates: Option<serde_json::Value>,
    pub min_size: Decimal,
    pub max_spread: Decimal,
    pub event_start_date: Option<String>,
    pub event_end_date: Option<String>,
    pub in_game_multiplier: Option<Decimal>,
    pub reward_epoch: Option<Decimal>,
}

#[derive(Debug, serde::Deserialize)]
pub struct OrderBookSummary {
    pub market: String,
    pub asset_id: String,
    pub hash: String,
    #[serde(deserialize_with = "crate::decode::deserializers::number_from_string")]
    pub timestamp: u64,
    pub bids: Vec<OrderSummary>,
    pub asks: Vec<OrderSummary>,
}

#[derive(Debug, serde::Deserialize)]
pub struct OrderSummary {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

#[derive(Debug, serde::Deserialize)]
pub struct MidpointResponse {
    pub mid: Decimal,
}

#[derive(Debug, serde::Deserialize)]
pub struct SpreadResponse {
    pub spread: Decimal,
}

#[derive(Debug, serde::Deserialize)]
pub struct PriceResponse {
    pub price: Decimal,
}

// Additional types for full compatibility with polymarket-rs-client
pub use crate::types::{ExtraOrderArgs, MarketOrderArgs};

#[derive(Debug, Default)]
pub struct CreateOrderOptions {
    pub tick_size: Option<Decimal>,
    pub neg_risk: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
pub struct TickSizeResponse {
    pub minimum_tick_size: Decimal,
}

#[derive(Debug, serde::Deserialize)]
pub struct NegRiskResponse {
    pub neg_risk: bool,
}

// Re-export for compatibility
pub type PolyfillClient = ClobClient; 