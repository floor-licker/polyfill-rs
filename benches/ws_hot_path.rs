//! Benchmarks for the WebSocket `book` hot path.
//!
//! These are intended to approximate a warmed, steady-state processing loop:
//! after init/warmup, per-message processing should avoid heap allocations.
//! The allocation checks live in `tests/no_alloc_hot_paths.rs`; these benches
//! focus on throughput/latency of the processing path.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use polyfill_rs::types::BookUpdate;
use polyfill_rs::{OrderBookManager, OrderSummary, WsBookUpdateProcessor};
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicU64, Ordering};

const START_TIMESTAMP: u64 = 1_000_000_000_000_000;
const BOOK_ASSET_ID: &str = "test_asset_id";
const BOOK_MARKET: &str = "0xabc";

struct TimestampRange {
    start: usize,
    end: usize,
}

impl TimestampRange {
    fn find(bytes: &[u8]) -> Self {
        let needle = b"\"timestamp\":";
        let Some(pos) = bytes.windows(needle.len()).position(|w| w == needle) else {
            panic!("timestamp field not found in WS template JSON");
        };

        let start = pos + needle.len();
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }

        if start == end {
            panic!("timestamp digits not found in WS template JSON");
        }

        Self { start, end }
    }

    fn write_fixed_width(&self, bytes: &mut [u8], mut value: u64) {
        let width = self.end - self.start;

        // Write digits right-to-left into the existing digit window.
        for idx in (0..width).rev() {
            let digit = (value % 10) as u8;
            bytes[self.start + idx] = b'0' + digit;
            value /= 10;
        }
    }
}

fn price_string_from_ticks(ticks: u32) -> String {
    let whole = ticks / 10_000;
    let frac = ticks % 10_000;
    format!("{whole}.{frac:04}")
}

fn build_book_update(levels_per_side: usize) -> BookUpdate {
    let mut bids = Vec::with_capacity(levels_per_side);
    let mut asks = Vec::with_capacity(levels_per_side);

    let size = Decimal::new(1_000_000, 4); // 100.0000

    for i in 0..levels_per_side {
        let bid_ticks = 7_500u32 - i as u32;
        let ask_ticks = 7_501u32 + i as u32;
        bids.push(OrderSummary {
            price: Decimal::new(bid_ticks as i64, 4),
            size,
        });
        asks.push(OrderSummary {
            price: Decimal::new(ask_ticks as i64, 4),
            size,
        });
    }

    BookUpdate {
        asset_id: BOOK_ASSET_ID.to_string(),
        market: BOOK_MARKET.to_string(),
        timestamp: 1,
        bids,
        asks,
        hash: None,
    }
}

fn build_ws_book_template(levels_per_side: usize) -> Vec<u8> {
    let mut json = String::new();

    json.push_str("{\"event_type\":\"book\",\"asset_id\":\"");
    json.push_str(BOOK_ASSET_ID);
    json.push_str("\",\"timestamp\":");
    json.push_str(&START_TIMESTAMP.to_string());
    json.push_str(",\"bids\":[");

    let size = "100.0000";
    for i in 0..levels_per_side {
        if i != 0 {
            json.push(',');
        }
        let bid_ticks = 7_500u32 - i as u32;
        let bid_price = price_string_from_ticks(bid_ticks);
        json.push_str("{\"price\":\"");
        json.push_str(&bid_price);
        json.push_str("\",\"size\":\"");
        json.push_str(size);
        json.push_str("\"}");
    }

    json.push_str("],\"asks\":[");
    for i in 0..levels_per_side {
        if i != 0 {
            json.push(',');
        }
        let ask_ticks = 7_501u32 + i as u32;
        let ask_price = price_string_from_ticks(ask_ticks);
        json.push_str("{\"price\":\"");
        json.push_str(&ask_price);
        json.push_str("\",\"size\":\"");
        json.push_str(size);
        json.push_str("\"}");
    }
    json.push_str("]}");

    json.into_bytes()
}

fn bench_ws_book_process_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_hot_path");

    for levels_per_side in [1usize, 16, 64] {
        let books = OrderBookManager::new(levels_per_side * 2);
        let _ = books.get_or_create_book(BOOK_ASSET_ID).unwrap();

        // Warm up: ensure all levels exist so the steady-state path doesn't allocate.
        let warmup_update = build_book_update(levels_per_side);
        books.apply_book_update(&warmup_update).unwrap();

        let template = build_ws_book_template(levels_per_side);
        let ts_range = TimestampRange::find(&template);

        let mut processor = WsBookUpdateProcessor::new(template.len());
        let mut warmup_msg = template.clone();
        processor
            .process_bytes(warmup_msg.as_mut_slice(), &books)
            .unwrap();

        let counter = AtomicU64::new(START_TIMESTAMP);

        group.throughput(Throughput::Bytes(template.len() as u64));
        group.bench_function(
            format!("process_bytes_levels_per_side_{levels_per_side}"),
            move |b| {
                b.iter_batched(
                    || {
                        let mut msg = template.clone();
                        let ts = counter.fetch_add(1, Ordering::Relaxed) + 1;
                        ts_range.write_fixed_width(msg.as_mut_slice(), ts);
                        msg
                    },
                    |mut msg| {
                        let stats = processor
                            .process_bytes(black_box(msg.as_mut_slice()), black_box(&books))
                            .unwrap();
                        black_box(stats);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_ws_book_process_bytes);
criterion_main!(benches);
