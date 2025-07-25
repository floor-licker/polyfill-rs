//! Benchmark for order book updates
//!
//! This benchmark measures the performance of order book operations
//! including delta application, price updates, and book maintenance.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use polyfill_rs::{
    book::OrderBook,
    types::{OrderDelta, Side},
};
use rust_decimal::{Decimal, Decimal as RustDecimal};
use rust_decimal_macros::dec;
use std::time::Instant;

fn bench_book_creation(c: &mut Criterion) {
    c.bench_function("book_creation", |b| {
        b.iter(|| {
            let _book = OrderBook::new(
                black_box("test_token".to_string()),
                black_box(100),
            );
        });
    });
}

fn bench_delta_application(c: &mut Criterion) {
    let mut book = OrderBook::new("test_token".to_string(), 100);
    
    // Pre-populate with some levels
    for i in 1..=10 {
        let price = Decimal::from(50 + i) / Decimal::from(100);
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: Side::BUY,
            price,
            size: dec!(100),
            sequence: i,
        };
        book.apply_delta(delta).unwrap();
    }

    c.bench_function("delta_application", |b| {
        b.iter(|| {
            let delta = OrderDelta {
                token_id: "test_token".to_string(),
                timestamp: chrono::Utc::now(),
                side: black_box(Side::SELL),
                price: black_box(dec!(0.52)),
                size: black_box(dec!(50)),
                sequence: black_box(11),
            };
            book.apply_delta(delta).unwrap();
        });
    });
}

fn bench_best_price_lookup(c: &mut Criterion) {
    let mut book = OrderBook::new("test_token".to_string(), 100);
    
    // Pre-populate with levels
    for i in 1..=20 {
        let price = Decimal::from(50 + i) / Decimal::from(100);
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
            price,
            size: dec!(100),
            sequence: i,
        };
        book.apply_delta(delta).unwrap();
    }

    c.bench_function("best_price_lookup", |b| {
        b.iter(|| {
            let _bid = book.best_bid();
            let _ask = book.best_ask();
            let _spread = book.spread();
            let _mid = book.mid_price();
        });
    });
}

fn bench_book_snapshot(c: &mut Criterion) {
    let mut book = OrderBook::new("test_token".to_string(), 100);
    
    // Pre-populate with levels
    for i in 1..=50 {
        let price = Decimal::from(50 + i) / Decimal::from(100);
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
            price,
            size: dec!(100),
            sequence: i,
        };
        book.apply_delta(delta).unwrap();
    }

    c.bench_function("book_snapshot", |b| {
        b.iter(|| {
            let _snapshot = book.snapshot();
        });
    });
}

fn bench_market_impact_calculation(c: &mut Criterion) {
    let mut book = OrderBook::new("test_token".to_string(), 100);
    
    // Pre-populate with levels
    for i in 1..=30 {
        let price = Decimal::from(50 + i) / Decimal::from(100);
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
            price,
            size: dec!(100),
            sequence: i,
        };
        book.apply_delta(delta).unwrap();
    }

    c.bench_function("market_impact_calculation", |b| {
        b.iter(|| {
            let _impact = book.calculate_market_impact(Side::BUY, dec!(50));
        });
    });
}

fn bench_high_frequency_updates(c: &mut Criterion) {
    c.bench_function("high_frequency_updates", |b| {
        b.iter(|| {
            let mut book = OrderBook::new("test_token".to_string(), 100);
            let start_time = Instant::now();
            
            // Simulate high-frequency updates
            for i in 1..=1000 {
                let price = Decimal::from(500 + (i % 100)) / Decimal::from(1000);
                let size = Decimal::from(10 + (i % 90));
                let delta = OrderDelta {
                    token_id: "test_token".to_string(),
                    timestamp: chrono::Utc::now(),
                    side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
                    price,
                    size,
                    sequence: i,
                };
                book.apply_delta(delta).unwrap();
                
                // Check prices every 10 updates
                if i % 10 == 0 {
                    let _bid = book.best_bid();
                    let _ask = book.best_ask();
                }
            }
            
            let duration = start_time.elapsed();
            black_box(duration);
        });
    });
}

fn bench_concurrent_access(c: &mut Criterion) {
    use std::sync::Arc;
    use tokio::sync::RwLock;
    
    c.bench_function("concurrent_access", |b| {
        b.iter(|| {
            let book = Arc::new(RwLock::new(OrderBook::new("test_token".to_string(), 100)));
            let book_clone = book.clone();
            
            // Simulate concurrent reads and writes
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut tasks = Vec::new();
                
                // Spawn writer tasks
                for i in 1..=10 {
                    let book = book.clone();
                    tasks.push(tokio::spawn(async move {
                        let price = Decimal::from(50 + i) / Decimal::from(100);
                        let delta = OrderDelta {
                            token_id: "test_token".to_string(),
                            timestamp: chrono::Utc::now(),
                            side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
                            price,
                            size: dec!(100),
                            sequence: i,
                        };
                        let mut book = book.write().await;
                        book.apply_delta(delta).unwrap();
                    }));
                }
                
                // Spawn reader tasks
                for _ in 0..20 {
                    let book = book_clone.clone();
                    tasks.push(tokio::spawn(async move {
                        let book = book.read().await;
                        let _bid = book.best_bid();
                        let _ask = book.best_ask();
                    }));
                }
                
                // Wait for all tasks
                for task in tasks {
                    let _ = task.await;
                }
            });
        });
    });
}

criterion_group!(
    benches,
    bench_book_creation,
    bench_delta_application,
    bench_best_price_lookup,
    bench_book_snapshot,
    bench_market_impact_calculation,
    bench_high_frequency_updates,
    bench_concurrent_access,
);
criterion_main!(benches); 