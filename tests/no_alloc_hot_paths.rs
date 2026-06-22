use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use chrono::Utc;
use polyfill_rs::{
    book::OrderBookManager, OrderBookImpl, Side, WebSocketStream, WsBookUpdateProcessor,
};
use rust_decimal::Decimal;

thread_local! {
    static HEAP_OPERATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HEAP_OPERATIONS.with(|count| count.set(count.get() + 1));
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        HEAP_OPERATIONS.with(|count| count.set(count.get() + 1));
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        HEAP_OPERATIONS.with(|count| count.set(count.get() + 1));
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HEAP_OPERATIONS.with(|count| count.set(count.get() + 1));
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn heap_operation_count() -> usize {
    HEAP_OPERATIONS.with(|count| count.get())
}

struct NoHeapTrafficGuard {
    before: usize,
}

impl NoHeapTrafficGuard {
    fn new() -> Self {
        Self {
            before: heap_operation_count(),
        }
    }

    fn assert_no_heap_traffic(self) {
        let after = heap_operation_count();
        assert_eq!(
            after,
            self.before,
            "expected no heap traffic, but saw {} allocator operation(s)",
            after - self.before
        );
    }
}

#[test]
fn allocator_counter_tracks_deallocations() {
    let vec = Vec::<u8>::with_capacity(1024);
    let before = heap_operation_count();
    drop(vec);
    let after = heap_operation_count();

    assert!(
        after > before,
        "expected dropping an allocated Vec to count as heap traffic"
    );
}

fn token_id_hash(token_id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    token_id.hash(&mut hasher);
    hasher.finish()
}

fn mk_delta(
    token_id_hash: u64,
    side: Side,
    price_ticks: polyfill_rs::types::Price,
    size_units: polyfill_rs::types::Qty,
    sequence: u64,
) -> polyfill_rs::types::FastOrderDelta {
    polyfill_rs::types::FastOrderDelta {
        token_id_hash,
        timestamp: chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        side,
        price: price_ticks,
        size: size_units,
        sequence,
    }
}

#[test]
fn no_alloc_mid_and_spread_fast() {
    let token_id = "test_token";
    let token_hash = token_id_hash(token_id);
    let mut book = OrderBookImpl::new(token_id.to_string(), 100);

    // Allocate during setup: create initial price levels.
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 1_000_000, 1))
        .unwrap();
    book.apply_delta_fast(mk_delta(token_hash, Side::SELL, 7600, 1_000_000, 2))
        .unwrap();

    // Warm up allocator-counter TLS access before measuring (defensive).
    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    assert!(book.best_bid_fast().is_some());
    assert!(book.best_ask_fast().is_some());
    assert!(book.spread_fast().is_some());
    assert!(book.mid_price_fast().is_some());
    guard.assert_no_heap_traffic();
}

#[test]
fn no_alloc_book_analysis_fast_paths() {
    let token_id = "test_token";
    let token_hash = token_id_hash(token_id);
    let mut book = OrderBookImpl::new(token_id.to_string(), 100);

    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 1_000_000, 1))
        .unwrap();
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7400, 500_000, 2))
        .unwrap();
    book.apply_delta_fast(mk_delta(token_hash, Side::SELL, 7600, 800_000, 3))
        .unwrap();
    book.apply_delta_fast(mk_delta(token_hash, Side::SELL, 7700, 1_200_000, 4))
        .unwrap();

    let impact_size = Decimal::from_str("150.0").unwrap();
    let min_price = Decimal::from_str("0.74").unwrap();
    let max_price = Decimal::from_str("0.77").unwrap();
    let min_average_price = Decimal::from_str("0.76").unwrap();
    let expected_buy_liquidity = Decimal::from_str("200.0").unwrap();
    let expected_sell_liquidity = Decimal::from_str("150.0").unwrap();

    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    let impact = book
        .calculate_market_impact(Side::BUY, impact_size)
        .unwrap();
    assert!(impact.average_price > min_average_price);
    assert_eq!(
        book.liquidity_in_range(min_price, max_price, Side::BUY),
        expected_buy_liquidity
    );
    assert_eq!(
        book.liquidity_in_range(min_price, max_price, Side::SELL),
        expected_sell_liquidity
    );
    assert!(book.is_valid());
    guard.assert_no_heap_traffic();
}

#[test]
fn no_alloc_apply_delta_fast_existing_level_update() {
    let token_id = "test_token";
    let token_hash = token_id_hash(token_id);
    let mut book = OrderBookImpl::new(token_id.to_string(), 100);

    // Allocate during setup: create an initial level.
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 1_000_000, 1))
        .unwrap();

    // Warm up allocator-counter TLS access before measuring (defensive).
    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    // Updating an existing level should not touch the heap allocator.
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 2_000_000, 2))
        .unwrap();
    guard.assert_no_heap_traffic();
}

#[test]
fn no_alloc_apply_book_update_existing_levels() {
    let asset_id = "test_asset_id";
    let token_hash = token_id_hash(asset_id);
    let mut book = OrderBookImpl::new(asset_id.to_string(), 100);

    // Allocate during setup: create initial price levels.
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 1_000_000, 1))
        .unwrap();
    book.apply_delta_fast(mk_delta(token_hash, Side::SELL, 7600, 1_000_000, 2))
        .unwrap();

    let update = polyfill_rs::types::BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp: 10,
        bids: vec![polyfill_rs::types::OrderSummary {
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str("200.0").unwrap(),
        }],
        asks: vec![polyfill_rs::types::OrderSummary {
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str("50.0").unwrap(),
        }],
        hash: None,
    };

    // Warm up allocator-counter TLS access before measuring (defensive).
    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    book.apply_book_update(&update).unwrap();
    guard.assert_no_heap_traffic();
}

#[test]
fn no_alloc_book_manager_apply_book_update_existing_levels() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    // Warm up the internal book with initial levels (allocator traffic allowed).
    manager
        .apply_delta(polyfill_rs::types::OrderDelta {
            token_id: asset_id.to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::BUY,
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str("100.0").unwrap(),
            sequence: 1,
        })
        .unwrap();
    manager
        .apply_delta(polyfill_rs::types::OrderDelta {
            token_id: asset_id.to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::SELL,
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str("100.0").unwrap(),
            sequence: 2,
        })
        .unwrap();

    let update = polyfill_rs::types::BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp: 10,
        bids: vec![polyfill_rs::types::OrderSummary {
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str("200.0").unwrap(),
        }],
        asks: vec![polyfill_rs::types::OrderSummary {
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str("50.0").unwrap(),
        }],
        hash: None,
    };

    // Warm up allocator-counter TLS access before measuring (defensive).
    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    manager.apply_book_update(&update).unwrap();
    guard.assert_no_heap_traffic();
}

#[test]
fn no_alloc_ws_book_update_processor_apply_existing_levels() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    // Warm up the internal book with initial levels (allocator traffic allowed).
    manager
        .apply_delta(polyfill_rs::types::OrderDelta {
            token_id: asset_id.to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::BUY,
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str("100.0").unwrap(),
            sequence: 1,
        })
        .unwrap();
    manager
        .apply_delta(polyfill_rs::types::OrderDelta {
            token_id: asset_id.to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::SELL,
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str("100.0").unwrap(),
            sequence: 2,
        })
        .unwrap();

    let mut processor = WsBookUpdateProcessor::new(1024);

    // Warm up simd-json buffers/tape outside the guarded section.
    let mut warmup_msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200.0\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50.0\"}}]}}"
    )
    .into_bytes();
    processor
        .process_bytes(warmup_msg.as_mut_slice(), &manager)
        .unwrap();

    let mut msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"150.0\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75.0\"}}]}}"
    )
    .into_bytes();

    // Warm up allocator-counter TLS access before measuring (defensive).
    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    processor
        .process_bytes(msg.as_mut_slice(), &manager)
        .unwrap();
    guard.assert_no_heap_traffic();
}

#[test]
fn no_alloc_websocket_book_applier_apply_bytes_message_existing_levels() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    // Warm up the internal book with initial levels (allocator traffic allowed).
    manager
        .apply_delta(polyfill_rs::types::OrderDelta {
            token_id: asset_id.to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::BUY,
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str("100.0").unwrap(),
            sequence: 1,
        })
        .unwrap();
    manager
        .apply_delta(polyfill_rs::types::OrderDelta {
            token_id: asset_id.to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::SELL,
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str("100.0").unwrap(),
            sequence: 2,
        })
        .unwrap();

    let processor = WsBookUpdateProcessor::new(1024);
    let stream = WebSocketStream::new("wss://example.com/ws");
    let mut applier = stream.into_book_applier(&manager, processor);

    // Warm up simd-json buffers/tape outside the guarded section.
    let mut warmup_msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200.0\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50.0\"}}]}}"
    )
    .into_bytes();
    applier
        .apply_bytes_message(warmup_msg.as_mut_slice())
        .unwrap();

    let mut msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"150.0\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75.0\"}}]}}"
    )
    .into_bytes();

    // Warm up allocator-counter TLS access before measuring (defensive).
    let _ = heap_operation_count();

    let guard = NoHeapTrafficGuard::new();
    applier.apply_bytes_message(msg.as_mut_slice()).unwrap();
    guard.assert_no_heap_traffic();
}
