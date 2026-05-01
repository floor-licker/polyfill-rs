//! Zero-allocation-ish WebSocket hot-path processing.
//!
//! This module is focused on the "decode + apply" path for WS `book` events:
//! after warmup, processing a message should not perform heap allocations.
//!
//! Important: using the current tokio-tungstenite transport, the *network layer*
//! may still allocate when producing `Message::Text(String)`. This module aims to
//! make the *processing* layer allocation-free so we can enforce it with tests.

use crate::book::OrderBookManager;
use crate::errors::{PolyfillError, Result};
use crate::types::{Price, Qty, Side, MAX_QTY};
use simd_json::prelude::*;

const WS_DECIMAL_SCALE_DIGITS: usize = 4;
const WS_SCALE_FACTOR: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseScaledError {
    Empty,
    InvalidChar,
    MultipleDots,
    TooManyDecimals,
    Overflow,
    ZeroPrice,
}

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

        simd_json::fill_tape(bytes, &mut self.buffers, &mut tape).map_err(|e| {
            PolyfillError::parse("Failed to parse WebSocket JSON", Some(Box::new(e)))
        })?;

        let root = tape.as_value();
        let stats = process_root_value(root, books)?;

        // Reset the tape to detach lifetimes and keep capacity for reuse.
        self.tape = Some(tape.reset());
        Ok(stats)
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
) -> Result<WsBookApplyStats> {
    if let Some(obj) = value.as_object() {
        return process_stream_object(obj, books);
    }

    let Some(arr) = value.as_array() else {
        return Ok(WsBookApplyStats::default());
    };

    let mut total = WsBookApplyStats::default();
    for elem in arr.iter() {
        let Some(obj) = elem.as_object() else {
            continue;
        };
        let stats = process_stream_object(obj, books)?;
        total.book_messages += stats.book_messages;
        total.book_levels_applied += stats.book_levels_applied;
    }

    Ok(total)
}

fn process_stream_object<'tape, 'input>(
    obj: simd_json::tape::Object<'tape, 'input>,
    books: &OrderBookManager,
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

    let bids = obj.get("bids").and_then(|v| v.as_array());
    let asks = obj.get("asks").and_then(|v| v.as_array());

    let levels_applied = books.with_book_mut(asset_id, |book| {
        if !book.begin_ws_book_update(asset_id, timestamp)? {
            return Ok(0);
        }

        let mut applied = 0usize;
        if let Some(bids) = bids {
            applied += apply_levels(book, Side::BUY, bids)?;
        }
        if let Some(asks) = asks {
            applied += apply_levels(book, Side::SELL, asks)?;
        }

        book.finish_ws_book_update();
        Ok(applied)
    })?;

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

#[inline]
fn parse_price_ticks_ascii(s: &str) -> std::result::Result<Price, ParseScaledError> {
    let ticks = parse_scaled_ascii_u64(s, Price::MAX as u64)?;
    if ticks == 0 {
        return Err(ParseScaledError::ZeroPrice);
    }

    Ok(ticks as Price)
}

#[inline]
fn parse_qty_units_ascii(s: &str) -> std::result::Result<Qty, ParseScaledError> {
    let units = parse_scaled_ascii_u64(s, MAX_QTY as u64)?;
    Ok(units as Qty)
}

#[inline]
fn parse_scaled_ascii_u64(s: &str, max: u64) -> std::result::Result<u64, ParseScaledError> {
    if s.is_empty() {
        return Err(ParseScaledError::Empty);
    }

    let mut int = 0u64;
    let mut frac = 0u64;
    let mut frac_digits = 0usize;
    let mut seen_dot = false;
    let mut seen_digit = false;

    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                seen_digit = true;
                let d = u64::from(b - b'0');
                if seen_dot {
                    if frac_digits < WS_DECIMAL_SCALE_DIGITS {
                        frac = frac
                            .checked_mul(10)
                            .and_then(|x| x.checked_add(d))
                            .ok_or(ParseScaledError::Overflow)?;
                        frac_digits += 1;
                    } else if d != 0 {
                        return Err(ParseScaledError::TooManyDecimals);
                    }
                } else {
                    int = int
                        .checked_mul(10)
                        .and_then(|x| x.checked_add(d))
                        .ok_or(ParseScaledError::Overflow)?;
                }
            },
            b'.' if !seen_dot => {
                seen_dot = true;
            },
            b'.' => return Err(ParseScaledError::MultipleDots),
            _ => return Err(ParseScaledError::InvalidChar),
        }
    }

    if !seen_digit {
        return Err(ParseScaledError::Empty);
    }

    while frac_digits < WS_DECIMAL_SCALE_DIGITS {
        frac = frac.checked_mul(10).ok_or(ParseScaledError::Overflow)?;
        frac_digits += 1;
    }

    let scaled = int
        .checked_mul(WS_SCALE_FACTOR)
        .and_then(|x| x.checked_add(frac))
        .ok_or(ParseScaledError::Overflow)?;

    if scaled > max {
        return Err(ParseScaledError::Overflow);
    }

    Ok(scaled)
}

fn apply_levels<'tape, 'input>(
    book: &mut crate::book::OrderBook,
    side: Side,
    levels: simd_json::tape::Array<'tape, 'input>,
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

        let price_ticks = parse_price_ticks_ascii(price_str)
            .map_err(|_| PolyfillError::validation("Invalid price"))?;
        let size_units = parse_qty_units_ascii(size_str)
            .map_err(|_| PolyfillError::validation("Invalid size"))?;

        book.apply_ws_book_level_fast(side, price_ticks, size_units)?;
        applied += 1;
    }

    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::{parse_price_ticks_ascii, parse_qty_units_ascii, ParseScaledError};
    use crate::{OrderBookManager, WsBookUpdateProcessor};

    #[test]
    fn parses_fixed_scale_prices_without_decimal() {
        assert_eq!(parse_price_ticks_ascii("0.7134"), Ok(7134));
        assert_eq!(parse_price_ticks_ascii("12.5000"), Ok(125_000));
        assert_eq!(parse_price_ticks_ascii("12.5"), Ok(125_000));
        assert_eq!(parse_price_ticks_ascii("1"), Ok(10_000));
        assert_eq!(parse_price_ticks_ascii(".5"), Ok(5_000));
        assert_eq!(parse_price_ticks_ascii("1.230000"), Ok(12_300));
    }

    #[test]
    fn parses_fixed_scale_sizes_without_decimal() {
        assert_eq!(parse_qty_units_ascii("0"), Ok(0));
        assert_eq!(parse_qty_units_ascii("100.0000"), Ok(1_000_000));
        assert_eq!(parse_qty_units_ascii("12.5000"), Ok(125_000));
        assert_eq!(parse_qty_units_ascii("12.5"), Ok(125_000));
    }

    #[test]
    fn rejects_invalid_scaled_ascii_values() {
        assert_eq!(parse_price_ticks_ascii(""), Err(ParseScaledError::Empty));
        assert_eq!(parse_price_ticks_ascii("."), Err(ParseScaledError::Empty));
        assert_eq!(
            parse_price_ticks_ascii("0"),
            Err(ParseScaledError::ZeroPrice)
        );
        assert_eq!(
            parse_price_ticks_ascii("0.0000"),
            Err(ParseScaledError::ZeroPrice)
        );
        assert_eq!(
            parse_price_ticks_ascii("0.00005"),
            Err(ParseScaledError::TooManyDecimals)
        );
        assert_eq!(
            parse_price_ticks_ascii("1.23456"),
            Err(ParseScaledError::TooManyDecimals)
        );
        assert_eq!(
            parse_price_ticks_ascii("1.2.3"),
            Err(ParseScaledError::MultipleDots)
        );
        assert_eq!(
            parse_qty_units_ascii("-1.0"),
            Err(ParseScaledError::InvalidChar)
        );
        assert_eq!(
            parse_qty_units_ascii("1e3"),
            Err(ParseScaledError::InvalidChar)
        );
        assert_eq!(
            parse_qty_units_ascii("922337203685478.0"),
            Err(ParseScaledError::Overflow)
        );
    }

    #[test]
    fn ws_book_processor_rejects_fractional_values_that_would_round() {
        let asset_id = "test_asset_id";
        let manager = OrderBookManager::new(10);
        manager.get_or_create_book(asset_id).unwrap();

        let mut processor = WsBookUpdateProcessor::new(1024);
        let mut msg = format!(
            "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75005\",\"size\":\"200.0\"}}],\"asks\":[]}}"
        )
        .into_bytes();

        let err = processor
            .process_bytes(msg.as_mut_slice(), &manager)
            .unwrap_err();
        assert!(err.to_string().contains("Invalid price"));
    }
}
