use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use chrono::Utc;
use polyfill_rs::{OrderBookImpl, Side};
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
