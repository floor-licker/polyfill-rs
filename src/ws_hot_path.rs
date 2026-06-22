//! Zero-allocation-ish WebSocket hot-path processing.
//!
//! This module is focused on the "decode + apply" path for WS `book` events:
//! after warmup, processing a message should not perform heap allocations.
//!
//! Important: using the current tokio-tungstenite transport, the *network layer*
//! may still allocate when producing `Message::Text(String)`. This module aims to
//! make the *processing* layer allocation-free so we can enforce it with tests.

use crate::book::{OrderBookManager, ParsedBookLevel};
use crate::errors::{PolyfillError, Result};
use crate::types::{Price, Qty, Side, MAX_PRICE_TICKS, MAX_QTY, MIN_PRICE_TICKS, SCALE_FACTOR};
use simd_json::prelude::*;

/// Summary of what happened while processing a WS payload.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WsBookApplyStats {
    pub book_messages: usize,
    pub book_levels_applied: usize,
}

/// In-place WS `book` message processor built on `simd-json`'s tape API.
///
/// This avoids building a DOM (which allocates for arrays/objects) by decoding into a
/// reusable tape, then traversing it to extract the fields needed for order book updates.
pub struct WsBookUpdateProcessor {
    buffers: simd_json::Buffers,
    tape: Option<simd_json::Tape<'static>>,
    parsed_levels: Vec<ParsedBookLevel>,
}

impl WsBookUpdateProcessor {
    /// Create a new processor.
    ///
    /// `input_len_hint` should be set to the typical WS message size to reduce warmup reallocs.
    pub fn new(input_len_hint: usize) -> Self {
        Self {
            buffers: simd_json::Buffers::new(input_len_hint),
            // Store an empty tape with a `'static` lifetime so we can reuse its allocation.
            tape: Some(simd_json::Tape::null().reset()),
            parsed_levels: Vec::with_capacity((input_len_hint / 32).max(8)),
        }
    }

    /// Process a WS payload in-place (bytes will be mutated by the JSON parser).
    pub fn process_bytes(
        &mut self,
        bytes: &mut [u8],
        books: &OrderBookManager,
    ) -> Result<WsBookApplyStats> {
        let mut tape = self
            .tape
            .take()
            .expect("WsBookUpdateProcessor tape must be present")
            .reset();

        let result = match simd_json::fill_tape(bytes, &mut self.buffers, &mut tape) {
            Ok(()) => {
                let root = tape.as_value();
                process_root_value(root, books, &mut self.parsed_levels)
            },
            Err(e) => Err(PolyfillError::parse(
                "Failed to parse WebSocket JSON",
                Some(Box::new(e)),
            )),
        };

        // Reset the tape to detach lifetimes and keep capacity for reuse.
        self.tape = Some(tape.reset());
        result
    }

    /// Convenience: process an owned text message without allocating an additional buffer.
    pub fn process_text(
        &mut self,
        text: String,
        books: &OrderBookManager,
    ) -> Result<WsBookApplyStats> {
        let mut bytes = text.into_bytes();
        self.process_bytes(bytes.as_mut_slice(), books)
    }
}

fn process_root_value<'tape, 'input>(
    value: simd_json::tape::Value<'tape, 'input>,
    books: &OrderBookManager,
    parsed_levels: &mut Vec<ParsedBookLevel>,
) -> Result<WsBookApplyStats> {
    if let Some(obj) = value.as_object() {
        return process_stream_object(obj, books, parsed_levels);
    }

    let Some(arr) = value.as_array() else {
        return Ok(WsBookApplyStats::default());
    };

    let mut total = WsBookApplyStats::default();
    for elem in arr.iter() {
        let Some(obj) = elem.as_object() else {
            continue;
        };
        let stats = process_stream_object(obj, books, parsed_levels)?;
        total.book_messages += stats.book_messages;
        total.book_levels_applied += stats.book_levels_applied;
    }

    Ok(total)
}

fn process_stream_object<'tape, 'input>(
    obj: simd_json::tape::Object<'tape, 'input>,
    books: &OrderBookManager,
    parsed_levels: &mut Vec<ParsedBookLevel>,
) -> Result<WsBookApplyStats> {
    let Some(event_type) = obj.get("event_type").and_then(|v| v.into_string()) else {
        return Ok(WsBookApplyStats::default());
    };

    if event_type != "book" {
        return Ok(WsBookApplyStats::default());
    }

    let asset_id = obj
        .get("asset_id")
        .and_then(|v| v.into_string())
        .ok_or_else(|| PolyfillError::parse("Missing asset_id", None))?;

    let timestamp_value = obj
        .get("timestamp")
        .ok_or_else(|| PolyfillError::parse("Missing timestamp", None))?;
    let timestamp = parse_u64(timestamp_value)
        .ok_or_else(|| PolyfillError::parse("Invalid timestamp", None))?;
    let hash = obj.get("hash").and_then(|v| v.into_string());

    let bids = obj
        .get("bids")
        .ok_or_else(|| PolyfillError::parse("Missing bids", None))?
        .as_array()
        .ok_or_else(|| PolyfillError::parse("Invalid bids", None))?;
    let asks = obj
        .get("asks")
        .ok_or_else(|| PolyfillError::parse("Missing asks", None))?
        .as_array()
        .ok_or_else(|| PolyfillError::parse("Invalid asks", None))?;

    let result = books.with_book_mut(asset_id, |book| {
        parsed_levels.clear();

        if !book.should_apply_ws_book_update(asset_id, timestamp, hash)? {
            return Ok(0);
        }

        collect_levels(Side::BUY, bids, parsed_levels)?;
        collect_levels(Side::SELL, asks, parsed_levels)?;

        let parsed_count = parsed_levels.len();
        if book.apply_ws_book_snapshot_fast(asset_id, timestamp, hash, parsed_levels)? {
            Ok(parsed_count)
        } else {
            Ok(0)
        }
    });
    parsed_levels.clear();
    let levels_applied = result?;

    Ok(WsBookApplyStats {
        book_messages: 1,
        book_levels_applied: levels_applied,
    })
}

fn parse_u64<'tape, 'input>(value: simd_json::tape::Value<'tape, 'input>) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.into_string().and_then(|s| s.parse::<u64>().ok()))
}

fn collect_levels<'tape, 'input>(
    side: Side,
    levels: simd_json::tape::Array<'tape, 'input>,
    parsed_levels: &mut Vec<ParsedBookLevel>,
) -> Result<usize> {
    let mut applied = 0usize;
    for level in levels.iter() {
        let Some(obj) = level.as_object() else {
            continue;
        };

        let price_str = obj
            .get("price")
            .and_then(|v| v.into_string())
            .ok_or_else(|| PolyfillError::parse("Missing price", None))?;
        let size_str = obj
            .get("size")
            .and_then(|v| v.into_string())
            .ok_or_else(|| PolyfillError::parse("Missing size", None))?;

        let price_ticks = parse_price_ticks_4dp(price_str)?;
        let size_units = parse_qty_scaled_4dp(size_str)?;

        parsed_levels.push(ParsedBookLevel {
            side,
            price_ticks,
            size_units,
        });
        applied += 1;
    }

    Ok(applied)
}

#[inline]
fn parse_price_ticks_4dp(value: &str) -> Result<Price> {
    let scaled = parse_scaled_4_u64(value)?;
    if scaled < MIN_PRICE_TICKS as u64 {
        return Err(PolyfillError::validation("Invalid price"));
    }
    if scaled > MAX_PRICE_TICKS as u64 {
        return Err(PolyfillError::validation("Invalid price"));
    }

    Ok(scaled as Price)
}

#[inline]
fn parse_qty_scaled_4dp(value: &str) -> Result<Qty> {
    let scaled = parse_scaled_4_u64(value)?;
    if scaled > MAX_QTY as u64 {
        return Err(PolyfillError::validation("Invalid size"));
    }

    Ok(scaled as Qty)
}

#[inline]
fn parse_scaled_4_u64(value: &str) -> Result<u64> {
    if value.is_empty() {
        return Err(PolyfillError::parse("invalid decimal", None));
    }

    let mut whole = 0u64;
    let mut frac = 0u64;
    let mut frac_digits = 0u8;
    let mut seen_dot = false;
    let mut seen_digit = false;

    for &byte in value.as_bytes() {
        match byte {
            b'0'..=b'9' => {
                seen_digit = true;
                let digit = (byte - b'0') as u64;
                if seen_dot {
                    if frac_digits >= 4 {
                        if digit != 0 {
                            return Err(PolyfillError::parse("too many decimal places", None));
                        }
                    } else {
                        frac = frac
                            .checked_mul(10)
                            .and_then(|x| x.checked_add(digit))
                            .ok_or_else(|| PolyfillError::parse("scaled value overflow", None))?;
                        frac_digits += 1;
                    }
                } else {
                    whole = whole
                        .checked_mul(10)
                        .and_then(|x| x.checked_add(digit))
                        .ok_or_else(|| PolyfillError::parse("scaled value overflow", None))?;
                }
            },
            b'.' if !seen_dot => {
                seen_dot = true;
            },
            _ => return Err(PolyfillError::parse("invalid decimal", None)),
        }
    }

    if !seen_digit {
        return Err(PolyfillError::parse("invalid decimal", None));
    }

    while frac_digits < 4 {
        frac *= 10;
        frac_digits += 1;
    }

    whole
        .checked_mul(SCALE_FACTOR as u64)
        .and_then(|x| x.checked_add(frac))
        .ok_or_else(|| PolyfillError::parse("scaled value overflow", None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BookUpdate, OrderSummary};
    use rust_decimal_macros::dec;

    #[test]
    fn fixed_point_parser_matches_expected_price_ticks() {
        assert_eq!(parse_price_ticks_4dp("0.6543").unwrap(), 6543);
        assert_eq!(parse_price_ticks_4dp("1.0000").unwrap(), 10_000);
        assert_eq!(parse_price_ticks_4dp("1.000000").unwrap(), 10_000);
        assert!(parse_price_ticks_4dp("0.00005").is_err());
        assert!(parse_price_ticks_4dp("0").is_err());
        assert!(parse_price_ticks_4dp("-0.1").is_err());
    }

    #[test]
    fn fixed_point_parser_matches_expected_qty_units() {
        assert_eq!(parse_qty_scaled_4dp("100.0").unwrap(), 1_000_000);
        assert_eq!(parse_qty_scaled_4dp("0.0000").unwrap(), 0);
        assert_eq!(parse_qty_scaled_4dp("1.234500").unwrap(), 12_345);
        assert!(parse_qty_scaled_4dp("-50.5").is_err());
        assert!(parse_qty_scaled_4dp("0.00004").is_err());
        assert!(parse_qty_scaled_4dp("0.00005").is_err());
    }

    #[test]
    fn processor_recovers_after_parse_and_validation_errors() {
        let books = OrderBookManager::new(10);
        books.get_or_create_book("test_asset_id").unwrap();
        let mut processor = WsBookUpdateProcessor::new(1024);

        let mut malformed_json = br#"{"event_type":"#.to_vec();
        assert!(processor
            .process_bytes(malformed_json.as_mut_slice(), &books)
            .is_err());

        let mut invalid_level = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1000,"bids":[{"price":"0.75001","size":"1.0000"}],"asks":[]}"#.to_vec();
        assert!(processor
            .process_bytes(invalid_level.as_mut_slice(), &books)
            .is_err());

        let mut valid_update = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1001,"bids":[{"price":"0.7500","size":"1.0000"}],"asks":[]}"#.to_vec();
        let stats = processor
            .process_bytes(valid_update.as_mut_slice(), &books)
            .unwrap();

        assert_eq!(stats.book_messages, 1);
        assert_eq!(stats.book_levels_applied, 1);
    }

    #[test]
    fn processor_error_keeps_existing_snapshot() {
        let books = OrderBookManager::new(10);
        books.get_or_create_book("test_asset_id").unwrap();
        books
            .apply_book_update(&BookUpdate {
                asset_id: "test_asset_id".to_string(),
                market: "0xabc".to_string(),
                timestamp: 1000,
                bids: vec![OrderSummary {
                    price: dec!(0.50),
                    size: dec!(10),
                }],
                asks: vec![OrderSummary {
                    price: dec!(0.60),
                    size: dec!(20),
                }],
                hash: None,
            })
            .unwrap();

        let mut processor = WsBookUpdateProcessor::new(1024);
        let mut invalid_snapshot = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1001,"bids":[{"price":"0.5100","size":"11.0000"},{"price":"0.51001","size":"12.0000"}],"asks":[{"price":"0.6100","size":"21.0000"}]}"#.to_vec();
        assert!(processor
            .process_bytes(invalid_snapshot.as_mut_slice(), &books)
            .is_err());

        let snapshot = books.get_book("test_asset_id").unwrap();
        assert_eq!(snapshot.sequence, 1000);
        assert_eq!(snapshot.bids.len(), 1);
        assert_eq!(snapshot.asks.len(), 1);
        assert_eq!(snapshot.bids[0].price, dec!(0.50));
        assert_eq!(snapshot.bids[0].size, dec!(10));
        assert_eq!(snapshot.asks[0].price, dec!(0.60));
        assert_eq!(snapshot.asks[0].size, dec!(20));
    }

    #[test]
    fn processor_missing_side_keeps_existing_snapshot() {
        let books = OrderBookManager::new(10);
        books.get_or_create_book("test_asset_id").unwrap();
        books
            .apply_book_update(&BookUpdate {
                asset_id: "test_asset_id".to_string(),
                market: "0xabc".to_string(),
                timestamp: 1000,
                bids: vec![OrderSummary {
                    price: dec!(0.50),
                    size: dec!(10),
                }],
                asks: vec![OrderSummary {
                    price: dec!(0.60),
                    size: dec!(20),
                }],
                hash: None,
            })
            .unwrap();

        let mut processor = WsBookUpdateProcessor::new(1024);
        let mut missing_asks = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1001,"bids":[{"price":"0.5100","size":"11.0000"}]}"#.to_vec();
        assert!(processor
            .process_bytes(missing_asks.as_mut_slice(), &books)
            .is_err());

        let snapshot = books.get_book("test_asset_id").unwrap();
        assert_eq!(snapshot.sequence, 1000);
        assert_eq!(snapshot.bids.len(), 1);
        assert_eq!(snapshot.asks.len(), 1);
        assert_eq!(snapshot.bids[0].price, dec!(0.50));
        assert_eq!(snapshot.asks[0].price, dec!(0.60));
    }

    #[test]
    fn processor_allows_same_timestamp_with_different_hash() {
        let books = OrderBookManager::new(10);
        books.get_or_create_book("test_asset_id").unwrap();
        let mut processor = WsBookUpdateProcessor::new(1024);

        let mut first = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1000,"hash":"hash_a","bids":[{"price":"0.5000","size":"10.0000"}],"asks":[{"price":"0.6000","size":"20.0000"}]}"#.to_vec();
        let first_stats = processor
            .process_bytes(first.as_mut_slice(), &books)
            .unwrap();
        assert_eq!(first_stats.book_levels_applied, 2);

        let mut second = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1000,"hash":"hash_b","bids":[{"price":"0.5100","size":"11.0000"}],"asks":[{"price":"0.6100","size":"21.0000"}]}"#.to_vec();
        let second_stats = processor
            .process_bytes(second.as_mut_slice(), &books)
            .unwrap();
        assert_eq!(second_stats.book_levels_applied, 2);

        let snapshot = books.get_book("test_asset_id").unwrap();
        assert_eq!(snapshot.sequence, 1000);
        assert_eq!(snapshot.bids[0].price, dec!(0.51));
        assert_eq!(snapshot.bids[0].size, dec!(11));
        assert_eq!(snapshot.asks[0].price, dec!(0.61));
        assert_eq!(snapshot.asks[0].size, dec!(21));

        let mut duplicate = br#"{"event_type":"book","asset_id":"test_asset_id","market":"0xabc","timestamp":1000,"hash":"hash_b","bids":[{"price":"0.5200","size":"12.0000"}],"asks":[{"price":"0.6200","size":"22.0000"}]}"#.to_vec();
        let duplicate_stats = processor
            .process_bytes(duplicate.as_mut_slice(), &books)
            .unwrap();
        assert_eq!(duplicate_stats.book_levels_applied, 0);

        let snapshot = books.get_book("test_asset_id").unwrap();
        assert_eq!(snapshot.bids[0].price, dec!(0.51));
        assert_eq!(snapshot.asks[0].price, dec!(0.61));
    }
}
