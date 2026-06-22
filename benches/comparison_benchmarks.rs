use alloy_signer_local::PrivateKeySigner;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use polyfill_rs::{
    auth::{create_l2_headers_with_body_bytes, PreparedApiCredentials},
    orders::{OrderBuilder, BYTES32_ZERO},
    types::{
        ApiCredentials, CreateOrderOptions, FastOrderDelta, OrderDelta, OrderType, PostOrder,
        PostOrderOptions,
    },
    OrderArgs, OrderBookImpl, Side,
};
use rust_decimal::Decimal;
use std::str::FromStr;

const TOKEN_ID: &str = "12345678901234567890";
const CHAIN_ID: u64 = 137;
const PRIVATE_KEY: &str = "0x1234567890123456789012345678901234567890123456789012345678901234";

fn test_signer() -> PrivateKeySigner {
    PRIVATE_KEY.parse().expect("valid benchmark private key")
}

fn test_order_args() -> OrderArgs {
    OrderArgs {
        token_id: TOKEN_ID.to_string(),
        price: Decimal::from_str("0.7537").unwrap(),
        size: Decimal::from_str("100.25").unwrap(),
        side: Side::BUY,
        expiration: Some(1_900_000_000),
        builder_code: Some(BYTES32_ZERO.to_string()),
        metadata: Some(BYTES32_ZERO.to_string()),
    }
}

fn test_order_options() -> CreateOrderOptions {
    CreateOrderOptions {
        tick_size: Some(Decimal::from_str("0.0001").unwrap()),
        neg_risk: Some(false),
    }
}

fn token_hash(token_id: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    token_id.hash(&mut hasher);
    hasher.finish()
}

fn fast_delta(token_id_hash: u64) -> FastOrderDelta {
    FastOrderDelta {
        token_id_hash,
        timestamp: chrono::Utc::now(),
        side: Side::BUY,
        price: 7_537,
        size: 1_002_500,
        sequence: 0,
    }
}

fn decimal_delta() -> OrderDelta {
    OrderDelta {
        token_id: TOKEN_ID.to_string(),
        timestamp: chrono::Utc::now(),
        side: Side::BUY,
        price: Decimal::from_str("0.7537").unwrap(),
        size: Decimal::from_str("100.25").unwrap(),
        sequence: 0,
    }
}

// Benchmark: Create and EIP-712 sign a limit order.
fn benchmark_create_order_eip712(c: &mut Criterion) {
    let signer = test_signer();
    let builder = OrderBuilder::new(signer, None, None);
    let order_args = test_order_args();
    let options = test_order_options();

    c.bench_function("create_order_eip712_signature", |b| {
        b.iter(|| {
            let signed_order = builder
                .create_order(
                    black_box(CHAIN_ID),
                    black_box(&order_args),
                    black_box(&options),
                )
                .unwrap();
            black_box(signed_order)
        })
    });
}

// Benchmark: Serialize a signed order body and build L2 auth headers for POST /order.
fn benchmark_order_submit_payload_auth(c: &mut Criterion) {
    let signer = test_signer();
    let builder = OrderBuilder::new(signer.clone(), None, None);
    let signed_order = builder
        .create_order(CHAIN_ID, &test_order_args(), &test_order_options())
        .unwrap();
    let post_options = PostOrderOptions {
        order_type: OrderType::GTD,
        post_only: false,
        defer_exec: false,
    };
    let api_creds = ApiCredentials {
        api_key: "benchmark-api-key".to_string(),
        secret: "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1".to_string(),
        passphrase: "benchmark-passphrase".to_string(),
    };
    let prepared_api_creds = PreparedApiCredentials::new(api_creds.clone());

    c.bench_function("order_submit_body_and_l2_headers", |b| {
        b.iter(|| {
            let body = PostOrder::new(
                black_box(signed_order.clone()),
                black_box(api_creds.api_key.clone()),
                black_box(post_options),
            );
            let body_bytes = serde_json::to_vec(black_box(&body)).unwrap();
            let headers = create_l2_headers_with_body_bytes(
                &signer,
                &prepared_api_creds,
                "POST",
                "/order",
                Some(&body_bytes),
            )
            .unwrap();
            black_box((body_bytes, headers))
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

// Benchmark: Core fixed-point order book update path.
fn benchmark_order_book_core_operations(c: &mut Criterion) {
    let token_id_hash = token_hash(TOKEN_ID);
    let mut book = OrderBookImpl::new(TOKEN_ID.to_string(), 100);
    let delta_template = fast_delta(token_id_hash);
    let mut sequence = 0;

    c.bench_function("order_book_apply_delta_fast_core", |b| {
        b.iter(|| {
            sequence += 1;
            let mut delta = delta_template;
            delta.sequence = sequence;
            book.apply_delta_fast(black_box(delta)).unwrap();
            black_box(book.sequence)
        })
    });
}

// Benchmark: External Decimal OrderDelta ingestion through the ergonomic API.
fn benchmark_order_book_external_ingestion(c: &mut Criterion) {
    let mut book = OrderBookImpl::new(TOKEN_ID.to_string(), 100);
    let delta_template = decimal_delta();
    let mut sequence = 0;

    c.bench_function("order_book_external_decimal_ingestion", |b| {
        b.iter(|| {
            sequence += 1;
            let mut delta = delta_template.clone();
            delta.sequence = sequence;
            book.apply_delta(black_box(delta)).unwrap();
            black_box(book.sequence)
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
    benchmark_order_submit_payload_auth,
    benchmark_json_parsing,
    benchmark_order_book_core_operations,
    benchmark_order_book_external_ingestion,
    benchmark_fast_operations
);
criterion_main!(benches);
