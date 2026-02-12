//! Coinbase message decoding
//!
//! Fast parsing of Coinbase WebSocket messages using SIMD-accelerated JSON
//! parsing when available, with serde_json fallback.

use crate::coinbase::types::*;
use crate::errors::{PolyfillError, Result};
use crate::types::{decimal_to_price, decimal_to_qty, Price, Qty, Side};
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

/// Parse a price string to fixed-point ticks
///
/// Coinbase sends prices as strings like "94123.45"
/// We convert to our internal tick representation (4 decimal places)
///
/// Note: BTC prices can be large (~$100k) but still fit in u32 ticks
/// Max representable: $429,496.7295 (sufficient for BTC)
pub fn parse_price(s: &str) -> Result<Price> {
    let decimal = Decimal::from_str(s)
        .map_err(|e| PolyfillError::parse(format!("Invalid price '{}': {}", s, e), None))?;
    decimal_to_price(decimal)
        .map_err(|e| PolyfillError::parse(format!("Price conversion failed: {}", e), None))
}

/// Parse a size string to fixed-point quantity
///
/// Coinbase sends sizes as strings like "0.00123456"
/// We convert to our internal representation (4 decimal places)
pub fn parse_size(s: &str) -> Result<Qty> {
    let decimal = Decimal::from_str(s)
        .map_err(|e| PolyfillError::parse(format!("Invalid size '{}': {}", s, e), None))?;
    decimal_to_qty(decimal)
        .map_err(|e| PolyfillError::parse(format!("Size conversion failed: {}", e), None))
}

/// Parse side string ("buy" or "sell") to Side enum
pub fn parse_side(s: &str) -> Result<Side> {
    match s {
        "buy" => Ok(Side::BUY),
        "sell" => Ok(Side::SELL),
        _ => Err(PolyfillError::parse(format!("Invalid side: {}", s), None)),
    }
}

/// Parse a raw Coinbase WebSocket message
///
/// Uses SIMD-accelerated parsing when available via simd_json,
/// falls back to serde_json for compatibility
pub fn parse_message(bytes: &mut [u8]) -> Result<Message> {
    // Try SIMD parsing first (2-3x faster)
    let value: Value = match simd_json::serde::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => {
            // Fallback to serde_json
            serde_json::from_slice(bytes)
                .map_err(|e| PolyfillError::parse(format!("JSON parse error: {}", e), None))?
        },
    };

    parse_message_value(value)
}

/// Parse from a JSON Value (useful when message is already parsed)
pub fn parse_message_value(value: Value) -> Result<Message> {
    let type_str = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PolyfillError::parse("Missing 'type' field", None))?;

    match type_str {
        "snapshot" => {
            let snapshot: Snapshot = serde_json::from_value(value)
                .map_err(|e| PolyfillError::parse(format!("Invalid snapshot: {}", e), None))?;
            Ok(Message::Snapshot(snapshot))
        },
        "l2update" => {
            let update: L2Update = serde_json::from_value(value)
                .map_err(|e| PolyfillError::parse(format!("Invalid l2update: {}", e), None))?;
            Ok(Message::L2Update(update))
        },
        "heartbeat" => {
            let heartbeat: Heartbeat = serde_json::from_value(value)
                .map_err(|e| PolyfillError::parse(format!("Invalid heartbeat: {}", e), None))?;
            Ok(Message::Heartbeat(heartbeat))
        },
        "subscriptions" => {
            let subs: Subscriptions = serde_json::from_value(value)
                .map_err(|e| PolyfillError::parse(format!("Invalid subscriptions: {}", e), None))?;
            Ok(Message::Subscriptions(subs))
        },
        "error" => {
            let err: ErrorMessage = serde_json::from_value(value)
                .map_err(|e| PolyfillError::parse(format!("Invalid error message: {}", e), None))?;
            Ok(Message::Error(err))
        },
        _ => {
            // Unknown message type - treat as heartbeat to avoid breaking
            Ok(Message::Heartbeat(Heartbeat {
                product_id: None,
                sequence: None,
                time: None,
            }))
        },
    }
}

/// Convert a Snapshot to fast internal format
///
/// Parses all price/size strings to fixed-point integers upfront
/// so orderbook operations can use pure integer math.
///
/// Note: Extreme prices (>$429k) that exceed u32 limits are silently skipped.
/// These are deep book levels irrelevant for trading (e.g., BTC asks at $500k when price is $94k).
pub fn snapshot_to_fast(snapshot: &Snapshot) -> Result<FastSnapshot> {
    let mut bids = Vec::with_capacity(snapshot.bids.len());
    for (price_str, size_str) in &snapshot.bids {
        match (parse_price(price_str), parse_size(size_str)) {
            (Ok(price), Ok(size)) => bids.push((price, size)),
            _ => {}, // Skip invalid/extreme prices silently
        }
    }

    let mut asks = Vec::with_capacity(snapshot.asks.len());
    for (price_str, size_str) in &snapshot.asks {
        match (parse_price(price_str), parse_size(size_str)) {
            (Ok(price), Ok(size)) => asks.push((price, size)),
            _ => {}, // Skip invalid/extreme prices silently
        }
    }

    Ok(FastSnapshot {
        product_id: snapshot.product_id.clone(),
        bids,
        asks,
    })
}

/// Convert an L2Update to fast internal format
///
/// Parses all changes to fixed-point integers for fast orderbook updates.
/// Extreme prices that exceed u32 limits are silently skipped.
pub fn l2update_to_fast(update: &L2Update) -> Result<FastL2Update> {
    let mut changes = Vec::with_capacity(update.changes.len());
    for (side_str, price_str, size_str) in &update.changes {
        match (
            parse_side(side_str),
            parse_price(price_str),
            parse_size(size_str),
        ) {
            (Ok(side), Ok(price), Ok(size)) => {
                changes.push(FastDelta { side, price, size });
            },
            _ => {}, // Skip invalid/extreme prices silently
        }
    }

    Ok(FastL2Update {
        product_id: update.product_id.clone(),
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_price() {
        // BTC price around $94,000
        let price = parse_price("94123.45").unwrap();
        assert_eq!(price, 941234500); // 94123.45 * 10000

        // Minimum tick
        let price = parse_price("0.0001").unwrap();
        assert_eq!(price, 1);

        // $1
        let price = parse_price("1.0000").unwrap();
        assert_eq!(price, 10000);
    }

    #[test]
    fn test_parse_size() {
        // 1 BTC
        let size = parse_size("1.00000000").unwrap();
        assert_eq!(size, 10000); // 1.0 * 10000

        // Small amount
        let size = parse_size("0.00010000").unwrap();
        assert_eq!(size, 1);
    }

    #[test]
    fn test_parse_side() {
        assert_eq!(parse_side("buy").unwrap(), Side::BUY);
        assert_eq!(parse_side("sell").unwrap(), Side::SELL);
        assert!(parse_side("invalid").is_err());
    }
}
