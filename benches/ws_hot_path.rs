//! Benchmarks for the WebSocket `book` hot path.
//!
//! These include the warmed happy path plus focused stress cases for level churn,
//! multi-asset routing, timestamp edges, malformed input, and production-like
//! bursts. The allocation checks live in `tests/no_alloc_hot_paths.rs`; these
//! benches focus on throughput/latency of the processing path.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use polyfill_rs::types::BookUpdate;
use polyfill_rs::{OrderBookManager, OrderSummary, StreamMessage, WsBookUpdateProcessor};
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicU64, Ordering};

const START_TIMESTAMP: u64 = 1_000_000_000_000_000;
const BOOK_ASSET_ID: &str = "test_asset_id";
const BOOK_MARKET: &str = "0xabc";

#[derive(Clone, Copy)]
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

fn bench_shard_index(token_id: &str, shard_count: usize) -> usize {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in token_id.as_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash as usize) % shard_count
}

fn asset_ids_on_same_shard(count: usize, shard_count: usize) -> Vec<String> {
    let anchor = "bench_asset_0".to_string();
    let target_shard = bench_shard_index(&anchor, shard_count);
    let mut assets = vec![anchor];

    for idx in 1..20_000 {
        if assets.len() == count {
            break;
        }

        let asset_id = format!("bench_asset_{idx}");
        if bench_shard_index(&asset_id, shard_count) == target_shard {
            assets.push(asset_id);
        }
    }

    assert_eq!(assets.len(), count, "failed to find same-shard assets");
    assets
}

fn asset_ids_on_distinct_shards(count: usize, shard_count: usize) -> Vec<String> {
    assert!(count <= shard_count);
    let mut assets = Vec::with_capacity(count);
    let mut seen = vec![false; shard_count];

    for idx in 0..20_000 {
        if assets.len() == count {
            break;
        }

        let asset_id = format!("bench_asset_{idx}");
        let shard = bench_shard_index(&asset_id, shard_count);
        if !seen[shard] {
            seen[shard] = true;
            assets.push(asset_id);
        }
    }

    assert_eq!(assets.len(), count, "failed to find distinct-shard assets");
    assets
}

fn build_book_update(levels_per_side: usize) -> BookUpdate {
    build_book_update_for(BOOK_ASSET_ID, levels_per_side)
}

fn build_book_update_for(asset_id: &str, levels_per_side: usize) -> BookUpdate {
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
        asset_id: asset_id.to_string(),
        market: BOOK_MARKET.to_string(),
        timestamp: 1,
        bids,
        asks,
        hash: None,
    }
}

fn build_ws_book_template(levels_per_side: usize) -> Vec<u8> {
    build_ws_book_template_for(BOOK_ASSET_ID, BOOK_MARKET, levels_per_side, 0)
}

fn build_ws_book_template_for(
    asset_id: &str,
    market: &str,
    levels_per_side: usize,
    price_shift_ticks: u32,
) -> Vec<u8> {
    let mut json = String::new();

    json.push_str("{\"event_type\":\"book\",\"asset_id\":\"");
    json.push_str(asset_id);
    json.push_str("\",\"market\":\"");
    json.push_str(market);
    json.push_str("\",\"timestamp\":");
    json.push_str(&START_TIMESTAMP.to_string());
    json.push_str(",\"bids\":[");

    let size = "100.0000";
    for i in 0..levels_per_side {
        if i != 0 {
            json.push(',');
        }
        let bid_ticks = 7_500u32 - price_shift_ticks - i as u32;
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
        let ask_ticks = 7_501u32 + price_shift_ticks + i as u32;
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

fn build_missing_price_template() -> Vec<u8> {
    let mut json = String::new();

    json.push_str("{\"event_type\":\"book\",\"asset_id\":\"");
    json.push_str(BOOK_ASSET_ID);
    json.push_str("\",\"market\":\"");
    json.push_str(BOOK_MARKET);
    json.push_str("\",\"timestamp\":");
    json.push_str(&START_TIMESTAMP.to_string());
    json.push_str(",\"bids\":[{\"size\":\"100.0000\"}],\"asks\":[]}");

    json.into_bytes()
}

fn build_invalid_price_template() -> Vec<u8> {
    let mut json = String::new();

    json.push_str("{\"event_type\":\"book\",\"asset_id\":\"");
    json.push_str(BOOK_ASSET_ID);
    json.push_str("\",\"market\":\"");
    json.push_str(BOOK_MARKET);
    json.push_str("\",\"timestamp\":");
    json.push_str(&START_TIMESTAMP.to_string());
    json.push_str(",\"bids\":[{\"price\":\"0.75001\",\"size\":\"100.0000\"}],\"asks\":[]}");

    json.into_bytes()
}

fn build_missing_asset_id_template() -> Vec<u8> {
    br#"{"event_type":"book","market":"0xabc","timestamp":1000000000000000,"bids":[],"asks":[]}"#
        .to_vec()
}

fn build_same_millisecond_template(levels_per_side: usize) -> Vec<u8> {
    let mut template = build_ws_book_template(levels_per_side);
    let ts_range = TimestampRange::find(&template);
    ts_range.write_fixed_width(template.as_mut_slice(), START_TIMESTAMP + 1);
    template
}

fn warm_books(books: &OrderBookManager, assets: &[String], levels_per_side: usize) {
    for asset_id in assets {
        let _ = books.get_or_create_book(asset_id).unwrap();
        let warmup_update = build_book_update_for(asset_id, levels_per_side);
        books.apply_book_update(&warmup_update).unwrap();
    }
}

fn message_from_template(template: &[u8], ts_range: TimestampRange, timestamp: u64) -> Vec<u8> {
    let mut msg = template.to_vec();
    ts_range.write_fixed_width(msg.as_mut_slice(), timestamp);
    msg
}

fn bench_ws_book_process_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_hot_path");

    for levels_per_side in [1usize, 16, 64] {
        let hot_path_books = OrderBookManager::new(levels_per_side * 2);
        let _ = hot_path_books.get_or_create_book(BOOK_ASSET_ID).unwrap();

        // Warm up: ensure all levels exist so the steady-state path doesn't allocate.
        let warmup_update = build_book_update(levels_per_side);
        hot_path_books.apply_book_update(&warmup_update).unwrap();

        let template = build_ws_book_template(levels_per_side);
        let tape_template = template.clone();
        let ts_range = TimestampRange::find(&tape_template);

        let mut processor = WsBookUpdateProcessor::new(tape_template.len());
        let mut warmup_msg = tape_template.clone();
        processor
            .process_bytes(warmup_msg.as_mut_slice(), &hot_path_books)
            .unwrap();

        let counter = AtomicU64::new(START_TIMESTAMP);

        group.throughput(Throughput::Bytes(tape_template.len() as u64));
        group.bench_function(
            format!("tape_process_and_apply_levels_per_side_{levels_per_side}"),
            move |b| {
                b.iter_batched(
                    || {
                        let mut msg = tape_template.clone();
                        let ts = counter.fetch_add(1, Ordering::Relaxed) + 1;
                        ts_range.write_fixed_width(msg.as_mut_slice(), ts);
                        msg
                    },
                    |mut msg| {
                        let stats = processor
                            .process_bytes(
                                black_box(msg.as_mut_slice()),
                                black_box(&hot_path_books),
                            )
                            .unwrap();
                        black_box(stats);
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        // Baseline: serde_json DOM -> StreamMessage -> BookUpdate -> apply to books.
        //
        // This is representative of our "non-hot-path" decoding approach and provides
        // a direct comparison within the same benchmark.
        let serde_books = OrderBookManager::new(levels_per_side * 2);
        let _ = serde_books.get_or_create_book(BOOK_ASSET_ID).unwrap();
        serde_books.apply_book_update(&warmup_update).unwrap();

        let serde_template = template;
        let serde_ts_range = TimestampRange::find(&serde_template);
        let serde_counter = AtomicU64::new(START_TIMESTAMP);

        group.bench_function(
            format!("serde_decode_and_apply_levels_per_side_{levels_per_side}"),
            move |b| {
                b.iter_batched(
                    || {
                        let mut msg = serde_template.clone();
                        let ts = serde_counter.fetch_add(1, Ordering::Relaxed) + 1;
                        serde_ts_range.write_fixed_width(msg.as_mut_slice(), ts);
                        msg
                    },
                    |msg| {
                        let messages = polyfill_rs::decode::parse_stream_messages_bytes(black_box(
                            msg.as_slice(),
                        ))
                        .unwrap();

                        for message in messages {
                            if let StreamMessage::Book(update) = message {
                                serde_books.apply_book_update(&update).unwrap();
                            }
                        }
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_ws_book_price_level_churn(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_churn");
    let levels_per_side = 64usize;
    let books = OrderBookManager::new(levels_per_side * 2);
    let _ = books.get_or_create_book(BOOK_ASSET_ID).unwrap();
    books
        .apply_book_update(&build_book_update(levels_per_side))
        .unwrap();

    let templates = (0..32)
        .map(|idx| build_ws_book_template_for(BOOK_ASSET_ID, BOOK_MARKET, levels_per_side, idx * 8))
        .collect::<Vec<_>>();
    let ts_ranges = templates
        .iter()
        .map(|template| TimestampRange::find(template))
        .collect::<Vec<_>>();

    let mut processor = WsBookUpdateProcessor::new(templates[0].len());
    let counter = AtomicU64::new(START_TIMESTAMP);
    let mut template_idx = 0usize;

    group.throughput(Throughput::Bytes(templates[0].len() as u64));
    group.bench_function("price_level_churn_64_levels", |b| {
        b.iter_batched(
            || {
                let idx = template_idx;
                template_idx = (template_idx + 1) % templates.len();
                let ts = counter.fetch_add(1, Ordering::Relaxed) + 1;
                message_from_template(&templates[idx], ts_ranges[idx], ts)
            },
            |mut msg| {
                let stats = processor
                    .process_bytes(black_box(msg.as_mut_slice()), black_box(&books))
                    .unwrap();
                black_box(stats);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_ws_book_multi_asset_routing(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_multi_asset");
    let shard_count = 4usize;
    let levels_per_side = 16usize;

    for (bench_name, assets) in [
        (
            "same_shard_4_assets_16_levels",
            asset_ids_on_same_shard(4, shard_count),
        ),
        (
            "different_shards_4_assets_16_levels",
            asset_ids_on_distinct_shards(4, shard_count),
        ),
    ] {
        let books = OrderBookManager::with_shard_count(levels_per_side * 2, shard_count);
        warm_books(&books, &assets, levels_per_side);

        let templates = assets
            .iter()
            .map(|asset_id| build_ws_book_template_for(asset_id, BOOK_MARKET, levels_per_side, 0))
            .collect::<Vec<_>>();
        let ts_ranges = templates
            .iter()
            .map(|template| TimestampRange::find(template))
            .collect::<Vec<_>>();

        let mut processor = WsBookUpdateProcessor::new(templates[0].len());
        let counter = AtomicU64::new(START_TIMESTAMP);
        let mut template_idx = 0usize;

        group.throughput(Throughput::Bytes(templates[0].len() as u64));
        group.bench_function(bench_name, |b| {
            b.iter_batched(
                || {
                    let idx = template_idx;
                    template_idx = (template_idx + 1) % templates.len();
                    let ts = counter.fetch_add(1, Ordering::Relaxed) + 1;
                    message_from_template(&templates[idx], ts_ranges[idx], ts)
                },
                |mut msg| {
                    let stats = processor
                        .process_bytes(black_box(msg.as_mut_slice()), black_box(&books))
                        .unwrap();
                    black_box(stats);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_ws_book_timestamp_edges(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_timestamp_edges");
    let levels_per_side = 16usize;
    let books = OrderBookManager::new(levels_per_side * 2);
    let _ = books.get_or_create_book(BOOK_ASSET_ID).unwrap();

    let mut processor = WsBookUpdateProcessor::new(1024);
    let mut warmup_msg = build_same_millisecond_template(levels_per_side);
    processor
        .process_bytes(warmup_msg.as_mut_slice(), &books)
        .unwrap();

    let template = build_same_millisecond_template(levels_per_side);
    group.throughput(Throughput::Bytes(template.len() as u64));
    group.bench_function("same_millisecond_snapshot_rejection", |b| {
        b.iter_batched(
            || template.clone(),
            |mut msg| {
                let stats = processor
                    .process_bytes(black_box(msg.as_mut_slice()), black_box(&books))
                    .unwrap();
                black_box(stats);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_ws_book_malformed_inputs(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_malformed_inputs");

    for (bench_name, template, rewrite_timestamp) in [
        ("missing_asset_id", build_missing_asset_id_template(), false),
        ("missing_price", build_missing_price_template(), true),
        (
            "invalid_price_precision",
            build_invalid_price_template(),
            true,
        ),
    ] {
        let books = OrderBookManager::new(32);
        let _ = books.get_or_create_book(BOOK_ASSET_ID).unwrap();
        books.apply_book_update(&build_book_update(16)).unwrap();

        let ts_range = rewrite_timestamp.then(|| TimestampRange::find(&template));
        let counter = AtomicU64::new(START_TIMESTAMP);
        let mut processor = WsBookUpdateProcessor::new(template.len());

        group.throughput(Throughput::Bytes(template.len() as u64));
        group.bench_function(bench_name, |b| {
            b.iter_batched(
                || {
                    let mut msg = template.clone();
                    if let Some(ts_range) = ts_range {
                        let ts = counter.fetch_add(1, Ordering::Relaxed) + 1;
                        ts_range.write_fixed_width(msg.as_mut_slice(), ts);
                    }
                    msg
                },
                |mut msg| {
                    let result =
                        processor.process_bytes(black_box(msg.as_mut_slice()), black_box(&books));
                    black_box(result.is_err());
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_ws_book_bursts(c: &mut Criterion) {
    let mut group = c.benchmark_group("ws_book_bursts");
    let levels_per_side = 32usize;
    let shard_count = 4usize;
    let assets = asset_ids_on_distinct_shards(4, shard_count);
    let books = OrderBookManager::with_shard_count(levels_per_side * 2, shard_count);
    warm_books(&books, &assets, levels_per_side);

    let templates = (0..32)
        .map(|idx| {
            let asset_id = &assets[idx % assets.len()];
            let shift = ((idx % 8) as u32) * 4;
            build_ws_book_template_for(asset_id, BOOK_MARKET, levels_per_side, shift)
        })
        .collect::<Vec<_>>();
    let ts_ranges = templates
        .iter()
        .map(|template| TimestampRange::find(template))
        .collect::<Vec<_>>();

    let mut processor = WsBookUpdateProcessor::new(templates[0].len());
    let counter = AtomicU64::new(START_TIMESTAMP);

    group.throughput(Throughput::Elements(templates.len() as u64));
    group.bench_function("mixed_32_message_burst", |b| {
        b.iter_batched(
            || {
                templates
                    .iter()
                    .zip(ts_ranges.iter().copied())
                    .map(|(template, ts_range)| {
                        let ts = counter.fetch_add(1, Ordering::Relaxed) + 1;
                        message_from_template(template, ts_range, ts)
                    })
                    .collect::<Vec<_>>()
            },
            |mut messages| {
                let mut total_levels = 0usize;
                for msg in &mut messages {
                    let stats = processor
                        .process_bytes(black_box(msg.as_mut_slice()), black_box(&books))
                        .unwrap();
                    total_levels += stats.book_levels_applied;
                }
                black_box(total_levels);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_ws_book_process_bytes,
    bench_ws_book_price_level_churn,
    bench_ws_book_multi_asset_routing,
    bench_ws_book_timestamp_edges,
    bench_ws_book_malformed_inputs,
    bench_ws_book_bursts
);
criterion_main!(benches);
