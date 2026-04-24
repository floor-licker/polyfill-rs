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
use crate::types::{decimal_to_price, decimal_to_qty, Side};
use rust_decimal::Decimal;
use simd_json::prelude::*;
use std::str::FromStr;

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
        // Assert ordering before any state mutation so a panic leaves the book unchanged.
        // Mirrors the pre-mutation check in `OrderBook::apply_book_update` in `book.rs`.
        #[cfg(debug_assertions)]
        {
            if let Some(bids) = bids {
                assert_levels_sorted(bids, "bids");
            }
            if let Some(asks) = asks {
                assert_levels_sorted(asks, "asks");
            }
        }

        if !book.begin_ws_book_update(asset_id, timestamp)? {
            return Ok(0);
        }

        let mut applied = 0usize;
        // Capture the first apply error so sweep still runs below — otherwise a
        // mid-flight failure would leave the book zeroed by the mark phase.
        let apply_result = 'apply: {
            if let Some(bids) = bids {
                match apply_levels(book, Side::BUY, bids) {
                    Ok(n) => applied += n,
                    Err(e) => break 'apply Err(e),
                }
            }
            if let Some(asks) = asks {
                match apply_levels(book, Side::SELL, asks) {
                    Ok(n) => applied += n,
                    Err(e) => break 'apply Err(e),
                }
            }
            Ok(())
        };

        book.finish_ws_book_update();

        apply_result.map(|_| applied)
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

        let price_decimal =
            Decimal::from_str(price_str).map_err(|_| PolyfillError::validation("Invalid price"))?;
        let size_decimal =
            Decimal::from_str(size_str).map_err(|_| PolyfillError::validation("Invalid size"))?;

        let price_ticks = decimal_to_price(price_decimal)
            .map_err(|_| PolyfillError::validation("Invalid price"))?;
        let size_units =
            decimal_to_qty(size_decimal).map_err(|_| PolyfillError::validation("Invalid size"))?;

        book.apply_ws_book_level_fast(side, price_ticks, size_units)?;
        applied += 1;
    }

    Ok(applied)
}

/// Debug-only: verify levels arrive in ascending price order.
///
/// Runs BEFORE any book mutation so a panic leaves the book unchanged. Iterates
/// the simd-json tape in place (zero-alloc) and silently skips malformed entries —
/// the real apply loop surfaces those as `Result` errors.
#[cfg(debug_assertions)]
fn assert_levels_sorted<'tape, 'input>(
    levels: simd_json::tape::Array<'tape, 'input>,
    side_label: &str,
) {
    let mut prev: Option<crate::types::Price> = None;
    for level in levels.iter() {
        let Some(obj) = level.as_object() else {
            continue;
        };
        let Some(price_str) = obj.get("price").and_then(|v| v.into_string()) else {
            continue;
        };
        let Ok(price_dec) = Decimal::from_str(price_str) else {
            continue;
        };
        let Ok(price_ticks) = decimal_to_price(price_dec) else {
            continue;
        };
        if let Some(p) = prev {
            debug_assert!(
                price_ticks >= p,
                "CLOB `book` message: {side_label} not in ascending price order ({price_ticks} < {p})",
            );
        }
        prev = Some(price_ticks);
    }
}
