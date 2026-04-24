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
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn allocation_count() -> usize {
    ALLOCATIONS.with(|count| count.get())
}

struct NoAllocGuard {
    before: usize,
}

impl NoAllocGuard {
    fn new() -> Self {
        Self {
            before: allocation_count(),
        }
    }

    fn assert_no_allocations(self) {
        let after = allocation_count();
        assert_eq!(
            after,
            self.before,
            "expected no heap allocations, but saw {} allocation(s)",
            after - self.before
        );
    }
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

    // Warm up TLS access before measuring (defensive).
    let _ = allocation_count();

    let guard = NoAllocGuard::new();
    assert!(book.best_bid_fast().is_some());
    assert!(book.best_ask_fast().is_some());
    assert!(book.spread_fast().is_some());
    assert!(book.mid_price_fast().is_some());
    guard.assert_no_allocations();
}

#[test]
fn no_alloc_apply_delta_fast_existing_level_update() {
    let token_id = "test_token";
    let token_hash = token_id_hash(token_id);
    let mut book = OrderBookImpl::new(token_id.to_string(), 100);

    // Allocate during setup: create an initial level.
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 1_000_000, 1))
        .unwrap();

    // Warm up TLS access before measuring (defensive).
    let _ = allocation_count();

    let guard = NoAllocGuard::new();
    // Updating an existing level should not require heap allocation.
    book.apply_delta_fast(mk_delta(token_hash, Side::BUY, 7500, 2_000_000, 2))
        .unwrap();
    guard.assert_no_allocations();
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

    // Warm up TLS access before measuring (defensive).
    let _ = allocation_count();

    let guard = NoAllocGuard::new();
    book.apply_book_update(&update).unwrap();
    guard.assert_no_allocations();
}

#[test]
fn no_alloc_book_manager_apply_book_update_existing_levels() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    // Warm up the internal book with initial levels (allocations allowed).
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

    // Warm up TLS access before measuring (defensive).
    let _ = allocation_count();

    let guard = NoAllocGuard::new();
    manager.apply_book_update(&update).unwrap();
    guard.assert_no_allocations();
}

#[test]
fn no_alloc_ws_book_update_processor_apply_existing_levels() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    // Warm up the internal book with initial levels (allocations allowed).
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

    // Warm up TLS access before measuring (defensive).
    let _ = allocation_count();

    let guard = NoAllocGuard::new();
    processor
        .process_bytes(msg.as_mut_slice(), &manager)
        .unwrap();
    guard.assert_no_allocations();
}

#[test]
fn no_alloc_websocket_book_applier_apply_text_message_existing_levels() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    // Warm up the internal book with initial levels (allocations allowed).
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
    let warmup_msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200.0\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50.0\"}}]}}"
    );
    applier.apply_text_message(warmup_msg).unwrap();

    let msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"150.0\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75.0\"}}]}}"
    );

    // Warm up TLS access before measuring (defensive).
    let _ = allocation_count();

    let guard = NoAllocGuard::new();
    applier.apply_text_message(msg).unwrap();
    guard.assert_no_allocations();
}

/// Zero-alloc contract for snapshot replay: the same snapshot applied twice
/// must not allocate on the second apply (steady-state ladder).
#[test]
fn no_alloc_steady_state_snapshot_replay() {
    let asset_id = "test_asset_id";
    let mut book = OrderBookImpl::new(asset_id.to_string(), 100);

    let snapshot = polyfill2::types::BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp: 10,
        bids: vec![
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.74").unwrap(),
                size: Decimal::from_str("100").unwrap(),
            },
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.75").unwrap(),
                size: Decimal::from_str("200").unwrap(),
            },
        ],
        asks: vec![
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.76").unwrap(),
                size: Decimal::from_str("50").unwrap(),
            },
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.77").unwrap(),
                size: Decimal::from_str("30").unwrap(),
            },
        ],
        hash: None,
    };

    // Warmup allocations allowed.
    book.apply_book_update(&snapshot).unwrap();

    // Replay with a strictly larger timestamp so the stale-sequence check passes.
    // Clone allocates (bids/asks Vecs), but happens before the guard — intentional.
    let replay = polyfill2::types::BookUpdate {
        timestamp: 11,
        ..snapshot.clone()
    };

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    book.apply_book_update(&replay).unwrap();
    guard.assert_no_allocations();
}

/// Same price ladder, different sizes — the common real-market case. Must not allocate.
#[test]
fn no_alloc_same_ladder_different_sizes() {
    let asset_id = "test_asset_id";
    let mut book = OrderBookImpl::new(asset_id.to_string(), 100);

    let make_snapshot = |ts: u64, bid_size: &str, ask_size: &str| polyfill2::types::BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp: ts,
        bids: vec![polyfill2::types::OrderSummary {
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str(bid_size).unwrap(),
        }],
        asks: vec![polyfill2::types::OrderSummary {
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str(ask_size).unwrap(),
        }],
        hash: None,
    };

    // Warmup.
    book.apply_book_update(&make_snapshot(10, "100", "50"))
        .unwrap();

    // Same ladder, different sizes.
    let update = make_snapshot(11, "250", "75");

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    book.apply_book_update(&update).unwrap();
    guard.assert_no_allocations();
}

/// Steady-state snapshot replay through the simd-json hot path.
#[test]
fn no_alloc_same_ladder_via_ws_processor() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    let mut processor = WsBookUpdateProcessor::new(1024);

    // Warmup #1: decode + apply (allocates for simd-json buffers, BTreeMap nodes).
    let mut warmup = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"100\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50\"}}]}}"
    )
    .into_bytes();
    processor
        .process_bytes(warmup.as_mut_slice(), &manager)
        .unwrap();

    // Second message: same ladder, newer timestamp, different sizes. Must be zero-alloc.
    let mut msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75\"}}]}}"
    )
    .into_bytes();

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    processor
        .process_bytes(msg.as_mut_slice(), &manager)
        .unwrap();
    guard.assert_no_allocations();
}

/// Steady-state snapshot replay through the WebSocketStream book applier.
#[test]
fn no_alloc_same_ladder_via_ws_applier() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    let processor = WsBookUpdateProcessor::new(1024);
    let stream = WebSocketStream::new("wss://example.com/ws");
    let mut applier = stream.into_book_applier(&manager, processor);

    let warmup = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"100\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50\"}}]}}"
    );
    applier.apply_text_message(warmup).unwrap();

    let msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75\"}}]}}"
    );

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    applier.apply_text_message(msg).unwrap();
    guard.assert_no_allocations();
}
