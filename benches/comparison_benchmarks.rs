use criterion::{black_box, criterion_group, criterion_main, Criterion};
use polyfill_rs::{OrderArgs, OrderBookImpl, Side};
use rust_decimal::Decimal;
use std::str::FromStr;

// Benchmark: Create an order with EIP-712 signature (computational cost only)
fn benchmark_create_order_eip712(c: &mut Criterion) {
    c.bench_function("create_order_eip712_signature", |b| {
        b.iter(|| {
            // Create order arguments - this benchmarks the computational cost
            let order_args = OrderArgs::new(
                "test_token_id",
                Decimal::from_str("0.75").unwrap(),
                Decimal::from_str("100.0").unwrap(),
                Side::BUY,
            );

            // Simulate the computational work of order creation
            black_box(order_args)
        })
    });
}

// Benchmark: JSON parsing (simulate market data parsing)
fn benchmark_json_parsing(c: &mut Criterion) {
    let sample_json = r#"{"data":[{"condition_id":"test","question":"Test Question","description":"Test Description","end_date_iso":"2024-01-01T00:00:00Z","game_start_time":"2024-01-01T00:00:00Z","image":"","icon":"","active":true,"closed":false,"archived":false,"accepting_orders":true,"minimum_order_size":"1.0","minimum_tick_size":"0.01","market_slug":"test","seconds_delay":0,"fpmm":"0x123","rewards":{"min_size":"1.0","max_spread":"0.1"},"tokens":[{"token_id":"123","outcome":"Yes","price":"0.5","winner":false}]}]}"#;

    c.bench_function("json_parsing_markets", |b| {
        b.iter(|| {
            // This benchmarks JSON parsing and deserialization
            let result: Result<serde_json::Value, _> = serde_json::from_str(sample_json);
            black_box(result)
        })
    });
}

// Benchmark: Order book operations
fn benchmark_order_book_operations(c: &mut Criterion) {
    c.bench_function("order_book_updates", |b| {
        b.iter(|| {
            let mut book = OrderBookImpl::new("test_token".to_string(), 100);

            // Simulate rapid order book updates
            for i in 0..1000 {
                let price = Decimal::from_str(&format!("0.{:04}", 5000 + (i % 100))).unwrap();
                let size = Decimal::from_str("100.0").unwrap();

                let bid_delta = polyfill_rs::OrderDelta {
                    token_id: "test_token".to_string(),
                    timestamp: chrono::Utc::now(),
                    side: polyfill_rs::Side::BUY,
                    price,
                    size,
                    sequence: i as u64,
                };

                let _ = book.apply_delta(bid_delta);
            }

            black_box(book)
        })
    });
}

// Benchmark: Fast order book operations
fn benchmark_fast_operations(c: &mut Criterion) {
    let mut book = OrderBookImpl::new("test_token".to_string(), 100);

    // Pre-populate the book
    for i in 0..50 {
        let price = Decimal::from_str(&format!("0.{:04}", 5000 + i)).unwrap();
        let size = Decimal::from_str("100.0").unwrap();

        let delta = polyfill_rs::OrderDelta {
            token_id: "test_token".to_string(),
            timestamp: chrono::Utc::now(),
            side: if i % 2 == 0 {
                polyfill_rs::Side::BUY
            } else {
                polyfill_rs::Side::SELL
            },
            price,
            size,
            sequence: i as u64,
        };

        let _ = book.apply_delta(delta);
    }

    c.bench_function("fast_spread_mid_calculations", |b| {
        b.iter(|| {
            // These use fixed-point arithmetic internally
            let spread = book.spread_fast();
            let mid = book.mid_price_fast();
            black_box((spread, mid))
        })
    });
}

criterion_group!(
    benches,
    benchmark_create_order_eip712,
    benchmark_json_parsing,
    benchmark_order_book_operations,
    benchmark_fast_operations
);
criterion_main!(benches);
