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

        book.finish_ws_book_update(
            |price_ticks| ws_levels_contain_price(bids, price_ticks),
            |price_ticks| ws_levels_contain_price(asks, price_ticks),
        );
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

        let price_ticks = parse_price_ticks(price_str)
            .ok_or_else(|| PolyfillError::validation("Invalid price"))?;
        let size_units =
            parse_qty_units(size_str).ok_or_else(|| PolyfillError::validation("Invalid size"))?;

        book.apply_ws_book_level_fast(side, price_ticks, size_units)?;
        applied += 1;
    }

    Ok(applied)
}

fn ws_levels_contain_price<'tape, 'input>(
    levels: Option<simd_json::tape::Array<'tape, 'input>>,
    price_ticks: Price,
) -> bool {
    let Some(levels) = levels else {
        return false;
    };

    levels.iter().any(|level| {
        let Some(obj) = level.as_object() else {
            return false;
        };
        let Some(price_str) = obj.get("price").and_then(|v| v.into_string()) else {
            return false;
        };
        let Some(size_str) = obj.get("size").and_then(|v| v.into_string()) else {
            return false;
        };
        let Some(level_price_ticks) = parse_price_ticks(price_str) else {
            return false;
        };
        let Some(size_units) = parse_qty_units(size_str) else {
            return false;
        };

        size_units != 0 && level_price_ticks == price_ticks
    })
}

fn parse_price_ticks(value: &str) -> Option<Price> {
    let scaled = parse_scaled_i128(value)?;
    if scaled < 0 {
        return None;
    }
    if scaled < MIN_PRICE_TICKS as i128 {
        return Some(MIN_PRICE_TICKS);
    }
    if scaled > MAX_PRICE_TICKS as i128 {
        return None;
    }

    Some(scaled as Price)
}

fn parse_qty_units(value: &str) -> Option<Qty> {
    let scaled = parse_scaled_i128(value)?;
    if scaled < -(MAX_QTY as i128) || scaled > MAX_QTY as i128 {
        return None;
    }

    Some(scaled as Qty)
}

fn parse_scaled_i128(value: &str) -> Option<i128> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let bytes = value.as_bytes();
    let mut idx = 0usize;
    let mut sign = 1i128;

    match bytes[idx] {
        b'-' => {
            sign = -1;
            idx += 1;
        },
        b'+' => {
            idx += 1;
        },
        _ => {},
    }

    if idx == bytes.len() {
        return None;
    }

    let mut whole = 0i128;
    let mut frac = 0i128;
    let mut frac_digits = 0usize;
    let mut round_up = false;
    let mut seen_dot = false;
    let mut seen_digit = false;

    while idx < bytes.len() {
        match bytes[idx] {
            b'0'..=b'9' => {
                seen_digit = true;
                let digit = (bytes[idx] - b'0') as i128;
                if seen_dot {
                    if frac_digits < 4 {
                        frac = frac.checked_mul(10)?.checked_add(digit)?;
                        frac_digits += 1;
                    } else if frac_digits == 4 {
                        round_up = digit >= 5;
                        frac_digits += 1;
                    }
                } else {
                    whole = whole.checked_mul(10)?.checked_add(digit)?;
                }
            },
            b'.' if !seen_dot => {
                seen_dot = true;
            },
            _ => return None,
        }
        idx += 1;
    }

    if !seen_digit {
        return None;
    }

    while frac_digits < 4 {
        frac *= 10;
        frac_digits += 1;
    }

    let mut magnitude = whole.checked_mul(SCALE_FACTOR as i128)?.checked_add(frac)?;

    if round_up {
        magnitude = magnitude.checked_add(1)?;
    }

    Some(magnitude * sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_parser_matches_expected_price_ticks() {
        assert_eq!(parse_price_ticks("0.6543"), Some(6543));
        assert_eq!(parse_price_ticks("1.0000"), Some(10_000));
        assert_eq!(parse_price_ticks("0.00005"), Some(1));
        assert_eq!(parse_price_ticks("0"), Some(1));
        assert_eq!(parse_price_ticks("-0.1"), None);
    }

    #[test]
    fn fixed_point_parser_matches_expected_qty_units() {
        assert_eq!(parse_qty_units("100.0"), Some(1_000_000));
        assert_eq!(parse_qty_units("-50.5"), Some(-505_000));
        assert_eq!(parse_qty_units("0.00004"), Some(0));
        assert_eq!(parse_qty_units("0.00005"), Some(1));
        assert_eq!(parse_qty_units("-0.00005"), Some(-1));
    }
}
