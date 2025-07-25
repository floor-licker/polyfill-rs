//! High-performance Rust client for Polymarket
//! 
//! This module provides a production-ready client for interacting with
//! Polymarket, optimized for high-frequency trading environments.

use crate::errors::{PolyfillError, Result};
use crate::types::*;
use reqwest::Client;
use serde_json::Value;
use std::str::FromStr;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use chrono::{DateTime, Utc};

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
}

impl ClobClient {
    /// Create a new client
    pub fn new(host: &str) -> Self {
        Self {
            http_client: Client::new(),
            base_url: host.to_string(),
            chain_id: 137, // Default to Polygon
        }
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
#[derive(Debug)]
pub struct MarketOrderArgs {
    pub token_id: String,
    pub amount: Decimal,
}

#[derive(Debug)]
pub struct ExtraOrderArgs {
    pub fee_rate_bps: u32,
    pub nonce: alloy_primitives::U256,
    pub taker: String,
}

impl Default for ExtraOrderArgs {
    fn default() -> Self {
        Self {
            fee_rate_bps: 0,
            nonce: alloy_primitives::U256::ZERO,
            taker: "0x0000000000000000000000000000000000000000".to_string(),
        }
    }
}

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