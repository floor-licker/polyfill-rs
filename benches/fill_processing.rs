//! Benchmark for fill processing
//!
//! This benchmark measures the performance of trade execution and
//! fill processing operations.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use polyfill_rs::{
    book::OrderBook,
    fill::{FillEngine, FillProcessor},
    types::{FillEvent, MarketOrderRequest, OrderDelta, Side},
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::time::Instant;

fn bench_fill_engine_creation(c: &mut Criterion) {
    c.bench_function("fill_engine_creation", |b| {
        b.iter(|| {
            let _engine = FillEngine::new(black_box(dec!(1)), black_box(dec!(5)), black_box(10));
        });
    });
}

fn bench_market_order_execution(c: &mut Criterion) {
    let mut engine = FillEngine::new(dec!(1), dec!(5), 10);
    let mut book = OrderBook::new("test_token".to_string(), 100);

    // Pre-populate book with levels
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

    c.bench_function("market_order_execution", |b| {
        b.iter(|| {
            let request = MarketOrderRequest {
                token_id: "test_token".to_string(),
                side: black_box(Side::BUY),
                amount: black_box(dec!(50)),
                slippage_tolerance: Some(dec!(1.0)),
                client_id: Some("bench_order".to_string()),
            };

            let _result = engine.execute_market_order(&request, &book);
        });
    });
}

fn bench_fill_processor(c: &mut Criterion) {
    let mut processor = FillProcessor::new(1000);

    c.bench_function("fill_processor", |b| {
        b.iter(|| {
            let fill = FillEvent {
                id: "fill_1".to_string(),
                order_id: "order_1".to_string(),
                token_id: "test_token".to_string(),
                side: black_box(Side::BUY),
                price: black_box(dec!(0.5)),
                size: black_box(dec!(100)),
                timestamp: chrono::Utc::now(),
                maker_address: alloy_primitives::Address::ZERO,
                taker_address: alloy_primitives::Address::ZERO,
                fee: black_box(dec!(0.1)),
            };

            processor.process_fill(fill).unwrap();
        });
    });
}

fn bench_market_impact_calculation(c: &mut Criterion) {
    let mut book = OrderBook::new("test_token".to_string(), 100);

    // Pre-populate with realistic order book
    for i in 1..=30 {
        let price = Decimal::from(50 + i) / Decimal::from(100);
        let size = Decimal::from(100 + i * 10);
        let delta = OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
            price,
            size,
            sequence: i,
        };
        book.apply_delta(delta).unwrap();
    }

    c.bench_function("market_impact_calculation", |b| {
        b.iter(|| {
            let _impact = book.calculate_market_impact(Side::BUY, dec!(50));
            let _impact = book.calculate_market_impact(Side::SELL, dec!(50));
        });
    });
}

fn bench_high_frequency_fills(c: &mut Criterion) {
    c.bench_function("high_frequency_fills", |b| {
        b.iter(|| {
            let mut engine = FillEngine::new(dec!(1), dec!(2), 5);
            let mut book = OrderBook::new("test_token".to_string(), 100);
            let start_time = Instant::now();

            // Simulate high-frequency fill processing
            for i in 1..=100 {
                // Add some market depth
                let price = Decimal::from(500 + (i % 10)) / Decimal::from(1000);
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

                // Execute market orders
                if i % 5 == 0 {
                    let request = MarketOrderRequest {
                        token_id: "test_token".to_string(),
                        side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
                        amount: dec!(10),
                        slippage_tolerance: Some(dec!(1.0)),
                        client_id: Some(format!("order_{}", i)),
                    };

                    let _result = engine.execute_market_order(&request, &book);
                }
            }

            let duration = start_time.elapsed();
            black_box(duration);
        });
    });
}

fn bench_fill_statistics(c: &mut Criterion) {
    let mut engine = FillEngine::new(dec!(1), dec!(5), 10);

    // Add some fills
    for i in 1..=100 {
        let request = MarketOrderRequest {
            token_id: "test_token".to_string(),
            side: if i % 2 == 0 { Side::BUY } else { Side::SELL },
            amount: dec!(10),
            slippage_tolerance: Some(dec!(1.0)),
            client_id: Some(format!("order_{}", i)),
        };

        let mut book = OrderBook::new("test_token".to_string(), 100);
        book.apply_delta(OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: request.side.opposite(),
            price: dec!(0.5),
            size: dec!(100),
            sequence: i,
        })
        .unwrap();

        let _result = engine.execute_market_order(&request, &book);
    }

    c.bench_function("fill_statistics", |b| {
        b.iter(|| {
            let _stats = engine.get_stats();
        });
    });
}

criterion_group!(
    benches,
    bench_fill_engine_creation,
    bench_market_order_execution,
    bench_fill_processor,
    bench_market_impact_calculation,
    bench_high_frequency_fills,
    bench_fill_statistics,
);
criterion_main!(benches);
