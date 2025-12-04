use criterion::{black_box, criterion_group, criterion_main, Criterion};
use polyfill_rs::{ClobClient, OrderArgs, Side};
use rust_decimal::Decimal;
use std::str::FromStr;
use tokio::runtime::Runtime;

// Benchmark: Real network request to get simplified markets
fn benchmark_real_simplified_markets(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("real_fetch_simplified_markets", |b| {
        b.iter(|| {
            rt.block_on(async {
                let client = ClobClient::new("https://clob.polymarket.com");
                
                // This is the real network request + JSON parsing
                let result = client.get_sampling_simplified_markets(None).await;
                black_box(result)
            })
        })
    });
}

// Benchmark: Real network request to get full markets
fn benchmark_real_markets(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("real_fetch_markets", |b| {
        b.iter(|| {
            rt.block_on(async {
                let client = ClobClient::new("https://clob.polymarket.com");
                
                // This is the real network request + JSON parsing
                let result = client.get_sampling_markets(None).await;
                black_box(result)
            })
        })
    });
}

// Benchmark: Real order creation (requires API credentials)
fn benchmark_real_order_creation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Skip if no credentials available
    let private_key = std::env::var("POLYMARKET_PRIVATE_KEY").ok();
    if private_key.is_none() {
        println!("Skipping order creation benchmark - no POLYMARKET_PRIVATE_KEY env var");
        return;
    }
    
    c.bench_function("real_create_order_eip712", |b| {
        b.iter(|| {
            rt.block_on(async {
                let client = ClobClient::new("https://clob.polymarket.com");
                
                // Set up credentials
                if let Ok(_key) = std::env::var("POLYMARKET_PRIVATE_KEY") {
                    // This would require implementing credential setup
                    // let creds = ApiCredentials::from_private_key(&key)?;
                    // client.set_credentials(creds);
                }
                
                let order_args = OrderArgs::new(
                    "test_token_id",
                    Decimal::from_str("0.75").unwrap(),
                    Decimal::from_str("100.0").unwrap(),
                    Side::BUY,
                );
                
                // This is the real EIP-712 signing + network request
                let result = client.create_order(&order_args, None, None, None).await;
                black_box(result)
            })
        })
    });
}

criterion_group!(
    network_benches,
    benchmark_real_simplified_markets,
    benchmark_real_markets,
    benchmark_real_order_creation
);
criterion_main!(network_benches);
