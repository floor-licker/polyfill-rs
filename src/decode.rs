//! Data decoding utilities for Polymarket client
//!
//! This module provides high-performance decoding functions for various
//! data formats used in trading environments.

use crate::errors::{PolyfillError, Result};
use crate::types::*;
use alloy_primitives::{Address, U256};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::str::FromStr;

/// Fast string to number deserializers
pub mod deserializers {
    use super::*;
    use std::fmt::Display;

    /// Deserialize number from string or number
    pub fn number_from_string<'de, T, D>(deserializer: D) -> std::result::Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr + serde::Deserialize<'de> + Clone,
        <T as FromStr>::Err: Display,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Number(n) => {
                if let Some(v) = n.as_u64() {
                    T::deserialize(serde_json::Value::Number(serde_json::Number::from(v)))
                        .map_err(|_| serde::de::Error::custom("Failed to deserialize number"))
                } else if let Some(v) = n.as_f64() {
                    T::deserialize(serde_json::Value::Number(
                        serde_json::Number::from_f64(v).unwrap(),
                    ))
                    .map_err(|_| serde::de::Error::custom("Failed to deserialize number"))
                } else {
                    Err(serde::de::Error::custom("Invalid number format"))
                }
            },
            serde_json::Value::String(s) => s.parse::<T>().map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom("Expected number or string")),
        }
    }

    /// Deserialize optional number from string
    pub fn optional_number_from_string<'de, T, D>(
        deserializer: D,
    ) -> std::result::Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: FromStr + serde::Deserialize<'de> + Clone,
        <T as FromStr>::Err: Display,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::Number(n) => {
                if let Some(v) = n.as_u64() {
                    T::deserialize(serde_json::Value::Number(serde_json::Number::from(v)))
                        .map(Some)
                        .map_err(|_| serde::de::Error::custom("Failed to deserialize number"))
                } else if let Some(v) = n.as_f64() {
                    T::deserialize(serde_json::Value::Number(
                        serde_json::Number::from_f64(v).unwrap(),
                    ))
                    .map(Some)
                    .map_err(|_| serde::de::Error::custom("Failed to deserialize number"))
                } else {
                    Err(serde::de::Error::custom("Invalid number format"))
                }
            },
            serde_json::Value::String(s) => {
                if s.is_empty() {
                    Ok(None)
                } else {
                    s.parse::<T>().map(Some).map_err(serde::de::Error::custom)
                }
            },
            _ => Err(serde::de::Error::custom("Expected number, string, or null")),
        }
    }

    /// Deserialize DateTime from Unix timestamp
    pub fn datetime_from_timestamp<'de, D>(
        deserializer: D,
    ) -> std::result::Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let timestamp = number_from_string::<u64, D>(deserializer)?;
        DateTime::from_timestamp(timestamp as i64, 0)
            .ok_or_else(|| serde::de::Error::custom("Invalid timestamp"))
    }

    /// Deserialize optional DateTime from Unix timestamp
    pub fn optional_datetime_from_timestamp<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match optional_number_from_string::<u64, D>(deserializer)? {
            Some(timestamp) => DateTime::from_timestamp(timestamp as i64, 0)
                .map(Some)
                .ok_or_else(|| serde::de::Error::custom("Invalid timestamp")),
            None => Ok(None),
        }
    }

    /// Deserialize a vec that may be `null` (treat `null` as empty vec).
    pub fn vec_from_null<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: serde::Deserialize<'de>,
    {
        Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
    }

    /// Deserialize an optional Decimal from string/number/null.
    ///
    /// - `null` => `None`
    /// - `""` => `None`
    /// - invalid values => error
    pub fn optional_decimal_from_string<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(s) => {
                let s = s.trim();
                if s.is_empty() {
                    Ok(None)
                } else {
                    s.parse::<Decimal>()
                        .map(Some)
                        .map_err(serde::de::Error::custom)
                }
            }
            serde_json::Value::Number(n) => Decimal::from_str(&n.to_string())
                .map(Some)
                .map_err(serde::de::Error::custom),
            other => Err(serde::de::Error::custom(format!(
                "Expected decimal as string/number/null, got {other}"
            ))),
        }
    }

    /// Like `optional_decimal_from_string`, but returns `None` on parse errors.
    pub fn optional_decimal_from_string_default_on_error<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(s) => {
                let s = s.trim();
                if s.is_empty() {
                    Ok(None)
                } else {
                    Ok(s.parse::<Decimal>().ok())
                }
            }
            serde_json::Value::Number(n) => Ok(Decimal::from_str(&n.to_string()).ok()),
            _ => Ok(None),
        }
    }
}

/// Raw API response types for efficient parsing
#[derive(Debug, Deserialize)]
pub struct RawOrderBookResponse {
    pub market: String,
    pub asset_id: String,
    pub hash: String,
    #[serde(deserialize_with = "deserializers::number_from_string")]
    pub timestamp: u64,
    pub bids: Vec<RawBookLevel>,
    pub asks: Vec<RawBookLevel>,
}

#[derive(Debug, Deserialize)]
pub struct RawBookLevel {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct RawOrderResponse {
    pub id: String,
    pub status: String,
    pub market: String,
    pub asset_id: String,
    pub maker_address: String,
    pub owner: String,
    pub outcome: String,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    pub side: Side,
    #[serde(with = "rust_decimal::serde::str")]
    pub original_size: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size_matched: Decimal,
    #[serde(deserialize_with = "deserializers::number_from_string")]
    pub expiration: u64,
    #[serde(deserialize_with = "deserializers::number_from_string")]
    pub created_at: u64,
}

#[derive(Debug, Deserialize)]
pub struct RawTradeResponse {
    pub id: String,
    pub market: String,
    pub asset_id: String,
    pub side: Side,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub size: Decimal,
    pub maker_address: String,
    pub taker_address: String,
    #[serde(deserialize_with = "deserializers::number_from_string")]
    pub timestamp: u64,
}

#[derive(Debug, Deserialize)]
pub struct RawMarketResponse {
    pub condition_id: String,
    pub tokens: [RawToken; 2],
    pub active: bool,
    pub closed: bool,
    pub question: String,
    pub description: String,
    pub category: Option<String>,
    pub end_date_iso: Option<String>,
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_order_size: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub minimum_tick_size: Decimal,
}

#[derive(Debug, Deserialize)]
pub struct RawToken {
    pub token_id: String,
    pub outcome: String,
}

/// Decoder implementations for converting raw responses to client types
pub trait Decoder<T> {
    fn decode(&self) -> Result<T>;
}

impl Decoder<OrderBook> for RawOrderBookResponse {
    fn decode(&self) -> Result<OrderBook> {
        let timestamp = chrono::DateTime::from_timestamp(self.timestamp as i64, 0)
            .ok_or_else(|| PolyfillError::parse("Invalid timestamp".to_string(), None))?;

        let bids = self
            .bids
            .iter()
            .map(|level| BookLevel {
                price: level.price,
                size: level.size,
            })
            .collect();

        let asks = self
            .asks
            .iter()
            .map(|level| BookLevel {
                price: level.price,
                size: level.size,
            })
            .collect();

        Ok(OrderBook {
            token_id: self.asset_id.clone(),
            timestamp,
            bids,
            asks,
            sequence: 0, // TODO: Get from response if available
        })
    }
}

impl Decoder<Order> for RawOrderResponse {
    fn decode(&self) -> Result<Order> {
        let status = match self.status.as_str() {
            "LIVE" => OrderStatus::Live,
            "CANCELLED" => OrderStatus::Cancelled,
            "FILLED" => OrderStatus::Filled,
            "PARTIAL" => OrderStatus::Partial,
            "EXPIRED" => OrderStatus::Expired,
            _ => {
                return Err(PolyfillError::parse(
                    format!("Unknown order status: {}", self.status),
                    None,
                ))
            },
        };

        let created_at =
            chrono::DateTime::from_timestamp(self.created_at as i64, 0).ok_or_else(|| {
                PolyfillError::parse("Invalid created_at timestamp".to_string(), None)
            })?;

        let expiration = if self.expiration > 0 {
            Some(
                chrono::DateTime::from_timestamp(self.expiration as i64, 0).ok_or_else(|| {
                    PolyfillError::parse("Invalid expiration timestamp".to_string(), None)
                })?,
            )
        } else {
            None
        };

        Ok(Order {
            id: self.id.clone(),
            token_id: self.asset_id.clone(),
            side: self.side,
            price: self.price,
            original_size: self.original_size,
            filled_size: self.size_matched,
            remaining_size: self.original_size - self.size_matched,
            status,
            order_type: self.order_type,
            created_at,
            updated_at: created_at, // Use same as created for now
            expiration,
            client_id: None,
        })
    }
}

impl Decoder<FillEvent> for RawTradeResponse {
    fn decode(&self) -> Result<FillEvent> {
        let timestamp = chrono::DateTime::from_timestamp(self.timestamp as i64, 0)
            .ok_or_else(|| PolyfillError::parse("Invalid trade timestamp".to_string(), None))?;

        let maker_address = Address::from_str(&self.maker_address)
            .map_err(|e| PolyfillError::parse(format!("Invalid maker address: {}", e), None))?;

        let taker_address = Address::from_str(&self.taker_address)
            .map_err(|e| PolyfillError::parse(format!("Invalid taker address: {}", e), None))?;

        Ok(FillEvent {
            id: self.id.clone(),
            order_id: "".to_string(), // TODO: Get from response if available
            token_id: self.asset_id.clone(),
            side: self.side,
            price: self.price,
            size: self.size,
            timestamp,
            maker_address,
            taker_address,
            fee: Decimal::ZERO, // TODO: Calculate or get from response
        })
    }
}

impl Decoder<Market> for RawMarketResponse {
    fn decode(&self) -> Result<Market> {
        let tokens = [
            Token {
                token_id: self.tokens[0].token_id.clone(),
                outcome: self.tokens[0].outcome.clone(),
                price: Decimal::ZERO,
                winner: false,
            },
            Token {
                token_id: self.tokens[1].token_id.clone(),
                outcome: self.tokens[1].outcome.clone(),
                price: Decimal::ZERO,
                winner: false,
            },
        ];

        Ok(Market {
            condition_id: self.condition_id.clone(),
            tokens,
            rewards: crate::types::Rewards {
                rates: None,
                min_size: Decimal::ZERO,
                max_spread: Decimal::ONE,
                event_start_date: None,
                event_end_date: None,
                in_game_multiplier: None,
                reward_epoch: None,
            },
            min_incentive_size: None,
            max_incentive_spread: None,
            active: self.active,
            closed: self.closed,
            question_id: self.condition_id.clone(), // Use condition_id as fallback
            minimum_order_size: self.minimum_order_size,
            minimum_tick_size: self.minimum_tick_size,
            description: self.description.clone(),
            category: self.category.clone(),
            end_date_iso: self.end_date_iso.clone(),
            game_start_time: None,
            question: self.question.clone(),
            market_slug: format!("market-{}", self.condition_id), // Generate a slug
            seconds_delay: Decimal::ZERO,
            icon: String::new(),
            fpmm: String::new(),
            // Additional fields
            enable_order_book: false,
            archived: false,
            accepting_orders: false,
            accepting_order_timestamp: None,
            maker_base_fee: Decimal::ZERO,
            taker_base_fee: Decimal::ZERO,
            notifications_enabled: false,
            neg_risk: false,
            neg_risk_market_id: String::new(),
            neg_risk_request_id: String::new(),
            image: String::new(),
            is_50_50_outcome: false,
        })
    }
}

/// WebSocket message parsing (official `event_type` shape).
///
/// Polymarket WebSocket servers may send either a single JSON object or a batch array.
/// This parser is tolerant:
/// - Unknown/unsupported `event_type`s are ignored.
/// - Invalid entries inside a batch are skipped (do not fail the whole batch).
pub fn parse_stream_messages(raw: &str) -> Result<Vec<StreamMessage>> {
    parse_stream_messages_bytes(raw.as_bytes())
}

/// See `parse_stream_messages`.
pub fn parse_stream_messages_bytes(bytes: &[u8]) -> Result<Vec<StreamMessage>> {
    let value: Value = serde_json::from_slice(bytes)?;

    match value {
        Value::Object(map) => {
            let event_type = map.get("event_type").and_then(Value::as_str);
            match event_type {
                None => Ok(vec![]),
                Some(_) => {
                    let msg: StreamMessage = serde_json::from_value(Value::Object(map))?;
                    match msg {
                        StreamMessage::Unknown => Ok(vec![]),
                        other => Ok(vec![other]),
                    }
                }
            }
        }
        Value::Array(arr) => Ok(arr
            .into_iter()
            .filter_map(|elem| {
                let obj = elem.as_object()?;
                let event_type = obj.get("event_type").and_then(Value::as_str)?;
                // Skip unknown event types early (forward compatibility).
                match event_type {
                    "book"
                    | "price_change"
                    | "tick_size_change"
                    | "last_trade_price"
                    | "best_bid_ask"
                    | "new_market"
                    | "market_resolved"
                    | "trade"
                    | "order" => {}
                    _ => return None,
                }

                match serde_json::from_value::<StreamMessage>(Value::Object(obj.clone())) {
                    Ok(StreamMessage::Unknown) => None,
                    Ok(msg) => Some(msg),
                    Err(_) => None,
                }
            })
            .collect()),
        _ => Ok(vec![]),
    }
}

/// Batch parsing utilities for high-throughput scenarios
pub struct BatchDecoder {
    buffer: Vec<u8>,
}

impl BatchDecoder {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(8192),
        }
    }

    /// Parse multiple JSON objects from a byte stream
    pub fn parse_json_stream<T>(&mut self, data: &[u8]) -> Result<Vec<T>>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        self.buffer.extend_from_slice(data);
        let mut results = Vec::new();
        let mut start = 0;

        while let Some(end) = self.find_json_boundary(start) {
            let json_slice = &self.buffer[start..end];
            if let Ok(obj) = serde_json::from_slice::<T>(json_slice) {
                results.push(obj);
            }
            start = end;
        }

        // Keep remaining incomplete data
        if start > 0 {
            self.buffer.drain(0..start);
        }

        Ok(results)
    }

    /// Find the end of a JSON object in the buffer
    fn find_json_boundary(&self, start: usize) -> Option<usize> {
        let mut depth = 0;
        let mut in_string = false;
        let mut escaped = false;

        for (i, &byte) in self.buffer[start..].iter().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }

            match byte {
                b'\\' if in_string => escaped = true,
                b'"' => in_string = !in_string,
                b'{' if !in_string => depth += 1,
                b'}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(start + i + 1);
                    }
                },
                _ => {},
            }
        }

        None
    }
}

impl Default for BatchDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Optimized parsers for common data types
pub mod fast_parse {
    use super::*;

    /// Fast decimal parsing for prices
    #[inline]
    pub fn parse_decimal(s: &str) -> Result<Decimal> {
        Decimal::from_str(s)
            .map_err(|e| PolyfillError::parse(format!("Invalid decimal: {}", e), None))
    }

    /// Fast address parsing
    #[inline]
    pub fn parse_address(s: &str) -> Result<Address> {
        Address::from_str(s)
            .map_err(|e| PolyfillError::parse(format!("Invalid address: {}", e), None))
    }

    /// Fast JSON parsing using SIMD instructions when possible
    /// Falls back to serde_json if simd-json fails
    /// Note: This requires owned types (no borrowing from input)
    #[inline]
    pub fn parse_json_fast<T>(bytes: &mut [u8]) -> Result<T>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        // Try SIMD parsing first (2-3x faster)
        match simd_json::serde::from_slice(bytes) {
            Ok(val) => Ok(val),
            Err(_) => {
                // Fallback to standard serde_json for safety
                serde_json::from_slice(bytes)
                    .map_err(|e| PolyfillError::parse(format!("JSON parse error: {}", e), None))
            },
        }
    }

    /// Fast JSON parsing for immutable data
    #[inline]
    pub fn parse_json_fast_owned<T>(bytes: &[u8]) -> Result<T>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        // Make a mutable copy for SIMD parsing
        let mut data = bytes.to_vec();
        parse_json_fast(&mut data)
    }

    /// Fast U256 parsing
    #[inline]
    pub fn parse_u256(s: &str) -> Result<U256> {
        U256::from_str_radix(s, 10)
            .map_err(|e| PolyfillError::parse(format!("Invalid U256: {}", e), None))
    }

    /// Parse Side enum
    #[inline]
    pub fn parse_side(s: &str) -> Result<Side> {
        match s.to_uppercase().as_str() {
            "BUY" => Ok(Side::BUY),
            "SELL" => Ok(Side::SELL),
            _ => Err(PolyfillError::parse(format!("Invalid side: {}", s), None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_decimal() {
        let result = fast_parse::parse_decimal("123.456").unwrap();
        assert_eq!(result, Decimal::from_str("123.456").unwrap());
    }

    #[test]
    fn test_parse_side() {
        assert_eq!(fast_parse::parse_side("BUY").unwrap(), Side::BUY);
        assert_eq!(fast_parse::parse_side("sell").unwrap(), Side::SELL);
        assert!(fast_parse::parse_side("invalid").is_err());
    }

    #[test]
    fn test_batch_decoder() {
        let mut decoder = BatchDecoder::new();
        let data = r#"{"test":1}{"test":2}"#.as_bytes();

        let results: Vec<serde_json::Value> = decoder.parse_json_stream(data).unwrap();
        assert_eq!(results.len(), 2);
    }
}
