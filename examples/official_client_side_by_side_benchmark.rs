//! Side-by-side benchmark comparing polyfill-rs against Polymarket's
//! `rs-clob-client-v2`.
//!
//! Run with:
//! `cargo run --release --example official_client_side_by_side_benchmark --features official-client-benchmark`
//!
//! Optional env vars:
//! - `POLYMARKET_BENCH_HOST` defaults to `https://clob.polymarket.com`
//! - `POLYMARKET_BENCH_ITERATIONS` defaults to `20`
//! - `POLYMARKET_BENCH_WARMUPS` defaults to `3`
//! - `POLYMARKET_BENCH_PARSE_ITERATIONS` defaults to `200`
//! - `POLYMARKET_BENCH_DELAY_MS` defaults to `100`
//! - `POLYMARKET_BENCH_KEEPALIVE` defaults to `true`

use std::hint::black_box;
use std::time::{Duration, Instant};

use polyfill_rs::ClobClient;
use polymarket_client_sdk_v2::clob::types::response::{
    Page as OfficialPage, SimplifiedMarketResponse as OfficialSimplifiedMarketResponse,
};
use polymarket_client_sdk_v2::clob::{Client as OfficialClient, Config as OfficialConfig};

type BenchResult<T> = Result<T, Box<dyn std::error::Error>>;

const INITIAL_CURSOR: &str = "MA==";
const OFFICIAL_SDK_REV: &str = "8ba5008733c3c03e92041eef8b1cb8495dbed718";

#[derive(Debug, Clone, Copy)]
struct Sample {
    elapsed: Duration,
    count: usize,
}

#[derive(Debug, Clone, Copy)]
struct Stats {
    mean_ms: f64,
    std_dev_ms: f64,
    min_ms: f64,
    max_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
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

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    let rank = (percentile * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn calc_stats(times: &[Duration]) -> Option<Stats> {
    if times.is_empty() {
        return None;
    }

    let mut values: Vec<f64> = times
        .iter()
        .map(|duration| duration.as_micros() as f64 / 1000.0)
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mean_ms = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean_ms).powi(2))
        .sum::<f64>()
        / values.len() as f64;

    Some(Stats {
        mean_ms,
        std_dev_ms: variance.sqrt(),
        min_ms: values[0],
        max_ms: values[values.len() - 1],
        p50_ms: percentile(&values, 0.50),
        p95_ms: percentile(&values, 0.95),
        p99_ms: percentile(&values, 0.99),
    })
}

fn print_stats(name: &str, stats: Option<Stats>, successes: usize, attempts: usize) {
    println!("{name}:");
    match stats {
        Some(stats) => {
            println!(
                "  p50/p95/p99: {:.1} / {:.1} / {:.1} ms",
                stats.p50_ms, stats.p95_ms, stats.p99_ms
            );
            println!(
                "  mean:        {:.1} ms +/- {:.1} ms",
                stats.mean_ms, stats.std_dev_ms
            );
            println!(
                "  range:       {:.1} - {:.1} ms",
                stats.min_ms, stats.max_ms
            );
            println!("  success:     {successes}/{attempts}");
        },
        None => println!("  success:     0/{attempts}"),
    }
}

fn print_single(name: &str, sample: &Result<Sample, String>) {
    match sample {
        Ok(sample) => println!(
            "{name}: {:.1} ms ({} items)",
            sample.elapsed.as_micros() as f64 / 1000.0,
            sample.count
        ),
        Err(error) => println!("{name}: ERROR - {error}"),
    }
}

async fn time_polyfill_typed(client: &ClobClient) -> Result<Sample, String> {
    let start = Instant::now();
    let page = client
        .get_simplified_markets(Some(INITIAL_CURSOR))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Sample {
        elapsed: start.elapsed(),
        count: page.data.len(),
    })
}

async fn time_official_typed(client: &OfficialClient) -> Result<Sample, String> {
    let start = Instant::now();
    let page = client
        .simplified_markets(Some(INITIAL_CURSOR.to_string()))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Sample {
        elapsed: start.elapsed(),
        count: page.data.len(),
    })
}

async fn time_polyfill_cold(host: &str) -> Result<Sample, String> {
    let start = Instant::now();
    let client = ClobClient::new(host);
    let page = client
        .get_simplified_markets(Some(INITIAL_CURSOR))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Sample {
        elapsed: start.elapsed(),
        count: page.data.len(),
    })
}

async fn time_official_cold(host: &str) -> Result<Sample, String> {
    let start = Instant::now();
    let client =
        OfficialClient::new(host, OfficialConfig::default()).map_err(|error| error.to_string())?;
    let page = client
        .simplified_markets(Some(INITIAL_CURSOR.to_string()))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Sample {
        elapsed: start.elapsed(),
        count: page.data.len(),
    })
}

async fn time_polyfill_raw(client: &ClobClient, url: &str) -> Result<Sample, String> {
    let start = Instant::now();
    let bytes = client
        .http_client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .bytes()
        .await
        .map_err(|error| error.to_string())?;
    Ok(Sample {
        elapsed: start.elapsed(),
        count: bytes.len(),
    })
}

async fn time_reqwest_raw(client: &reqwest::Client, url: &str) -> Result<Sample, String> {
    let start = Instant::now();
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .bytes()
        .await
        .map_err(|error| error.to_string())?;
    Ok(Sample {
        elapsed: start.elapsed(),
        count: bytes.len(),
    })
}

async fn run_typed_pairs(
    polyfill_client: &ClobClient,
    official_client: &OfficialClient,
    iterations: usize,
    delay: Duration,
) -> (Vec<Duration>, Vec<Duration>) {
    let mut polyfill_times = Vec::with_capacity(iterations);
    let mut official_times = Vec::with_capacity(iterations);

    for i in 1..=iterations {
        let polyfill_first = i % 2 == 1;
        let (polyfill_result, official_result) = if polyfill_first {
            let polyfill_result = time_polyfill_typed(polyfill_client).await;
            tokio::time::sleep(delay).await;
            let official_result = time_official_typed(official_client).await;
            (polyfill_result, official_result)
        } else {
            let official_result = time_official_typed(official_client).await;
            tokio::time::sleep(delay).await;
            let polyfill_result = time_polyfill_typed(polyfill_client).await;
            (polyfill_result, official_result)
        };

        if let Ok(sample) = polyfill_result {
            polyfill_times.push(sample.elapsed);
        }
        if let Ok(sample) = official_result {
            official_times.push(sample.elapsed);
        }

        tokio::time::sleep(delay).await;
    }

    (polyfill_times, official_times)
}

async fn run_raw_pairs(
    polyfill_client: &ClobClient,
    official_style_client: &reqwest::Client,
    url: &str,
    iterations: usize,
    delay: Duration,
) -> (Vec<Duration>, Vec<Duration>) {
    let mut polyfill_times = Vec::with_capacity(iterations);
    let mut official_style_times = Vec::with_capacity(iterations);

    for i in 1..=iterations {
        let polyfill_first = i % 2 == 1;
        let (polyfill_result, official_style_result) = if polyfill_first {
            let polyfill_result = time_polyfill_raw(polyfill_client, url).await;
            tokio::time::sleep(delay).await;
            let official_style_result = time_reqwest_raw(official_style_client, url).await;
            (polyfill_result, official_style_result)
        } else {
            let official_style_result = time_reqwest_raw(official_style_client, url).await;
            tokio::time::sleep(delay).await;
            let polyfill_result = time_polyfill_raw(polyfill_client, url).await;
            (polyfill_result, official_style_result)
        };

        if let Ok(sample) = polyfill_result {
            polyfill_times.push(sample.elapsed);
        }
        if let Ok(sample) = official_style_result {
            official_style_times.push(sample.elapsed);
        }

        tokio::time::sleep(delay).await;
    }

    (polyfill_times, official_style_times)
}

async fn run_typed_warmups(
    polyfill_client: &ClobClient,
    official_client: &OfficialClient,
    warmups: usize,
    delay: Duration,
) {
    for _ in 0..warmups {
        let _ = time_polyfill_typed(polyfill_client).await;
        tokio::time::sleep(delay).await;
        let _ = time_official_typed(official_client).await;
        tokio::time::sleep(delay).await;
    }
}

fn official_style_http_client() -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("rs_clob_client"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("*/*"),
    );
    headers.insert(
        reqwest::header::CONNECTION,
        reqwest::header::HeaderValue::from_static("keep-alive"),
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    reqwest::Client::builder().default_headers(headers).build()
}

fn parse_polyfill_once(bytes: &[u8]) -> BenchResult<Duration> {
    let start = Instant::now();
    let page: polyfill_rs::types::SimplifiedMarketsResponse =
        polyfill_rs::decode::fast_parse::parse_json_fast_owned(bytes)?;
    black_box(page);
    Ok(start.elapsed())
}

fn parse_official_direct_once(bytes: &[u8]) -> BenchResult<Duration> {
    let start = Instant::now();
    let page: OfficialPage<OfficialSimplifiedMarketResponse> = serde_json::from_slice(bytes)?;
    black_box(page);
    Ok(start.elapsed())
}

fn parse_official_helper_once(bytes: &[u8]) -> BenchResult<Duration> {
    let start = Instant::now();
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let page: Option<OfficialPage<OfficialSimplifiedMarketResponse>> =
        serde_json::from_value(value)?;
    black_box(page);
    Ok(start.elapsed())
}

fn run_parse_samples<F>(
    bytes: &[u8],
    iterations: usize,
    mut parse_once: F,
) -> BenchResult<Vec<Duration>>
where
    F: FnMut(&[u8]) -> BenchResult<Duration>,
{
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        times.push(parse_once(bytes)?);
    }
    Ok(times)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::var("POLYMARKET_BENCH_HOST")
        .unwrap_or_else(|_| "https://clob.polymarket.com".to_string());
    let url = format!("{host}/simplified-markets?next_cursor={INITIAL_CURSOR}");
    let iterations = env_usize("POLYMARKET_BENCH_ITERATIONS", 20);
    let warmups = env_usize("POLYMARKET_BENCH_WARMUPS", 3);
    let parse_iterations = env_usize("POLYMARKET_BENCH_PARSE_ITERATIONS", 200);
    let delay = Duration::from_millis(env_u64("POLYMARKET_BENCH_DELAY_MS", 100));
    let keepalive = env_bool("POLYMARKET_BENCH_KEEPALIVE", true);

    println!("=======================================================");
    println!(" polyfill-rs vs rs-clob-client-v2 benchmark");
    println!("=======================================================");
    println!("Endpoint: {url}");
    println!("Iterations: {iterations}");
    println!("Warmups: {warmups}");
    println!("Parse iterations: {parse_iterations}");
    println!("Delay: {} ms", delay.as_millis());
    println!("polyfill-rs background keepalive: {keepalive}");
    println!("rs-clob-client-v2 rev: {OFFICIAL_SDK_REV}");
    println!();

    println!("Cold start: client construction + first typed request");
    println!("-------------------------------------------------------");
    let polyfill_cold = time_polyfill_cold(&host).await;
    tokio::time::sleep(delay).await;
    let official_cold = time_official_cold(&host).await;
    print_single("polyfill-rs", &polyfill_cold);
    print_single("rs-clob-client-v2", &official_cold);
    println!();

    let polyfill_client = ClobClient::new(&host);
    let official_client = OfficialClient::new(&host, OfficialConfig::default())?;
    let official_style_http = official_style_http_client()?;

    if keepalive {
        polyfill_client
            .start_keepalive(Duration::from_secs(30))
            .await;
    }

    run_typed_warmups(&polyfill_client, &official_client, warmups, delay).await;

    println!("Warm connection: first typed request after warmup");
    println!("-------------------------------------------------------");
    let polyfill_warm = time_polyfill_typed(&polyfill_client).await;
    tokio::time::sleep(delay).await;
    let official_warm = time_official_typed(&official_client).await;
    print_single("polyfill-rs", &polyfill_warm);
    print_single("rs-clob-client-v2", &official_warm);
    println!();

    println!("Steady state: typed client total time");
    println!("-------------------------------------------------------");
    let (polyfill_typed_times, official_typed_times) =
        run_typed_pairs(&polyfill_client, &official_client, iterations, delay).await;
    print_stats(
        "polyfill-rs",
        calc_stats(&polyfill_typed_times),
        polyfill_typed_times.len(),
        iterations,
    );
    println!();
    print_stats(
        "rs-clob-client-v2",
        calc_stats(&official_typed_times),
        official_typed_times.len(),
        iterations,
    );
    println!();

    println!("Steady state: network-only byte fetch");
    println!("-------------------------------------------------------");
    let _ = time_polyfill_raw(&polyfill_client, &url).await;
    tokio::time::sleep(delay).await;
    let _ = time_reqwest_raw(&official_style_http, &url).await;
    tokio::time::sleep(delay).await;
    let (polyfill_raw_times, official_style_raw_times) = run_raw_pairs(
        &polyfill_client,
        &official_style_http,
        &url,
        iterations,
        delay,
    )
    .await;
    print_stats(
        "polyfill-rs HTTP",
        calc_stats(&polyfill_raw_times),
        polyfill_raw_times.len(),
        iterations,
    );
    println!();
    print_stats(
        "rs-clob-client-v2-style HTTP",
        calc_stats(&official_style_raw_times),
        official_style_raw_times.len(),
        iterations,
    );
    println!();

    println!("CPU-only parse from cached payload");
    println!("-------------------------------------------------------");
    let payload = polyfill_client
        .http_client
        .get(&url)
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();
    println!("payload: {} bytes", payload.len());
    let polyfill_parse_times = run_parse_samples(&payload, parse_iterations, parse_polyfill_once)?;
    let official_direct_parse_times =
        run_parse_samples(&payload, parse_iterations, parse_official_direct_once)?;
    let official_helper_parse_times =
        run_parse_samples(&payload, parse_iterations, parse_official_helper_once)?;
    print_stats(
        "polyfill-rs typed parse",
        calc_stats(&polyfill_parse_times),
        polyfill_parse_times.len(),
        parse_iterations,
    );
    println!();
    print_stats(
        "rs-clob-client-v2 direct typed parse",
        calc_stats(&official_direct_parse_times),
        official_direct_parse_times.len(),
        parse_iterations,
    );
    println!();
    print_stats(
        "rs-clob-client-v2 request-helper parse",
        calc_stats(&official_helper_parse_times),
        official_helper_parse_times.len(),
        parse_iterations,
    );

    if keepalive {
        polyfill_client.stop_keepalive().await;
    }

    Ok(())
}
