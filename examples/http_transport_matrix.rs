//! Compare HTTP transport configurations against the Polymarket CLOB endpoint.
//!
//! Run with:
//! `cargo run --release --example http_transport_matrix`

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONNECTION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, ClientBuilder};
use std::time::{Duration, Instant};

const INITIAL_CURSOR: &str = "MA==";

#[derive(Clone, Copy)]
struct Variant {
    name: &'static str,
    build: fn() -> Result<Client, reqwest::Error>,
}

#[derive(Clone, Copy)]
struct Stats {
    mean_ms: f64,
    sd_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

fn official_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("rs_clob_client"));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn polyfill_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("polyfill-rs/", env!("CARGO_PKG_VERSION"))),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn polyfill_headers_no_connection() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(concat!("polyfill-rs/", env!("CARGO_PKG_VERSION"))),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

fn polyfill_builder() -> ClientBuilder {
    Client::builder()
        .no_proxy()
        .http2_adaptive_window(true)
        .http2_initial_stream_window_size(512 * 1024)
        .tcp_nodelay(true)
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(90))
}

fn light_polyfill_builder() -> ClientBuilder {
    Client::builder()
        .no_proxy()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(90))
}

fn build_polyfill_current() -> Result<Client, reqwest::Error> {
    polyfill_builder().build()
}

fn build_reqwest_default() -> Result<Client, reqwest::Error> {
    Client::builder().build()
}

fn build_official_headers_default() -> Result<Client, reqwest::Error> {
    Client::builder()
        .default_headers(official_headers())
        .build()
}

fn build_polyfill_official_headers() -> Result<Client, reqwest::Error> {
    polyfill_builder()
        .default_headers(official_headers())
        .build()
}

fn build_polyfill_headers() -> Result<Client, reqwest::Error> {
    polyfill_builder()
        .default_headers(polyfill_headers())
        .build()
}

fn build_polyfill_headers_no_connection() -> Result<Client, reqwest::Error> {
    polyfill_builder()
        .default_headers(polyfill_headers_no_connection())
        .build()
}

fn build_default_polyfill_headers() -> Result<Client, reqwest::Error> {
    Client::builder()
        .default_headers(polyfill_headers())
        .build()
}

fn build_polyfill_light() -> Result<Client, reqwest::Error> {
    light_polyfill_builder().build()
}

fn build_http1_official_headers() -> Result<Client, reqwest::Error> {
    Client::builder()
        .http1_only()
        .default_headers(official_headers())
        .build()
}

async fn fetch_once(client: &Client, url: &str) -> Result<(Duration, usize), reqwest::Error> {
    let start = Instant::now();
    let bytes = client.get(url).send().await?.bytes().await?;
    Ok((start.elapsed(), bytes.len()))
}

fn percentile(sorted_ms: &[f64], percentile: f64) -> f64 {
    if sorted_ms.is_empty() {
        return 0.0;
    }

    let idx = ((sorted_ms.len() - 1) as f64 * percentile).round() as usize;
    sorted_ms[idx.min(sorted_ms.len() - 1)]
}

fn calc_stats(samples: &[Duration]) -> Stats {
    let values: Vec<f64> = samples
        .iter()
        .map(|duration| duration.as_micros() as f64 / 1000.0)
        .collect();
    let mean_ms = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean_ms;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    let mut sorted = values;
    sorted.sort_by(|a, b| a.total_cmp(b));

    Stats {
        mean_ms,
        sd_ms: variance.sqrt(),
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        p99_ms: percentile(&sorted, 0.99),
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::var("POLYMARKET_BENCH_HOST")
        .unwrap_or_else(|_| "https://clob.polymarket.com".to_string());
    let url = format!("{host}/simplified-markets?next_cursor={INITIAL_CURSOR}");
    let iterations = env_usize("POLYMARKET_HTTP_MATRIX_ITERATIONS", 12);
    let warmups = env_usize("POLYMARKET_HTTP_MATRIX_WARMUPS", 2);
    let delay = Duration::from_millis(env_u64("POLYMARKET_HTTP_MATRIX_DELAY_MS", 100));

    let mut variants = vec![
        Variant {
            name: "polyfill-current",
            build: build_polyfill_current,
        },
        Variant {
            name: "reqwest-default",
            build: build_reqwest_default,
        },
        Variant {
            name: "official-headers-default",
            build: build_official_headers_default,
        },
        Variant {
            name: "polyfill-current-official-headers",
            build: build_polyfill_official_headers,
        },
        Variant {
            name: "polyfill-current-polyfill-headers",
            build: build_polyfill_headers,
        },
        Variant {
            name: "polyfill-current-polyfill-headers-no-connection",
            build: build_polyfill_headers_no_connection,
        },
        Variant {
            name: "reqwest-default-polyfill-headers",
            build: build_default_polyfill_headers,
        },
        Variant {
            name: "polyfill-light-no-h2-window-tuning",
            build: build_polyfill_light,
        },
        Variant {
            name: "http1-official-headers",
            build: build_http1_official_headers,
        },
    ];
    if let Ok(filter) = std::env::var("POLYMARKET_HTTP_MATRIX_FILTER") {
        let filters: Vec<_> = filter
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect();
        variants.retain(|variant| filters.iter().any(|filter| variant.name.contains(filter)));
    }

    println!("HTTP transport matrix");
    println!("Endpoint: {url}");
    println!("Iterations: {iterations}");
    println!("Warmups: {warmups}");
    println!("Delay: {} ms", delay.as_millis());
    println!();

    println!("Cold client construction + first byte fetch");
    println!("------------------------------------------------------------");
    for variant in &variants {
        let start = Instant::now();
        let client = (variant.build)()?;
        let (fetch_elapsed, bytes) = fetch_once(&client, &url).await?;
        let total = start.elapsed();
        println!(
            "{:<38} total {:>7.1} ms | fetch {:>7.1} ms | {bytes} bytes",
            variant.name,
            total.as_micros() as f64 / 1000.0,
            fetch_elapsed.as_micros() as f64 / 1000.0
        );
        tokio::time::sleep(delay).await;
    }
    println!();

    let clients: Vec<_> = variants
        .into_iter()
        .map(|variant| Ok((variant.name, (variant.build)()?)))
        .collect::<Result<Vec<_>, reqwest::Error>>()?;

    for _ in 0..warmups {
        for (_, client) in &clients {
            let _ = fetch_once(client, &url).await;
            tokio::time::sleep(delay).await;
        }
    }

    let mut samples = vec![Vec::with_capacity(iterations); clients.len()];

    for iteration in 0..iterations {
        for offset in 0..clients.len() {
            let idx = (iteration + offset) % clients.len();
            let (_, client) = &clients[idx];
            let (elapsed, bytes) = fetch_once(client, &url).await?;
            if bytes == 0 {
                eprintln!("empty response for {}", clients[idx].0);
            }
            samples[idx].push(elapsed);
            tokio::time::sleep(delay).await;
        }
    }

    println!("Warm steady-state byte fetch");
    println!("------------------------------------------------------------");
    for ((name, _), sample) in clients.iter().zip(samples.iter()) {
        let stats = calc_stats(sample);
        println!(
            "{name:<38} mean {:>7.1} +/- {:>5.1} ms | p50/p95/p99 {:>7.1} / {:>7.1} / {:>7.1} ms",
            stats.mean_ms, stats.sd_ms, stats.p50_ms, stats.p95_ms, stats.p99_ms
        );
    }

    Ok(())
}
