// WebSocket integration tests for polyfill-rs
//
// These tests connect to Polymarket's live WS endpoints and are ignored by default.
//
// Run with:
//   cargo test --all-features --test ws_integration_tests -- --ignored --nocapture --test-threads=1

#![cfg(feature = "stream")]

use futures::StreamExt;
use polyfill_rs::{ClobClient, OrderBookManager, WebSocketStream, WsBookUpdateProcessor};
use std::env;
use std::time::Duration;

const HOST: &str = "https://clob.polymarket.com";
const WS_MARKET_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const WS_USER_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/user";
const CHAIN_ID: u64 = 137;

fn load_private_key() -> String {
    dotenvy::dotenv().ok();
    env::var("POLYMARKET_PRIVATE_KEY").expect("POLYMARKET_PRIVATE_KEY must be set (env or .env)")
}

fn stability_secs(default_secs: u64) -> u64 {
    env::var("POLYFILL_WS_STABILITY_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default_secs)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_ws_market_book_applier_receives_book_update() {
    // Pick an active token ID so the market channel should produce data.
    let client = ClobClient::new(HOST);
    let markets = client
        .get_sampling_markets(None)
        .await
        .expect("failed to fetch markets");

    let token_id = markets
        .data
        .iter()
        .find(|m| m.active && !m.closed)
        .and_then(|m| m.tokens.first())
        .map(|t| t.token_id.clone())
        .expect("no active markets found");

    let books = OrderBookManager::new(256);
    books
        .get_or_create_book(&token_id)
        .expect("failed to create book");

    let mut ws = WebSocketStream::new(WS_MARKET_URL);
    ws.subscribe_market_channel(vec![token_id.clone()])
        .await
        .expect("failed to subscribe market channel");

    let processor = WsBookUpdateProcessor::new(256 * 1024);
    let mut applier = ws.into_book_applier(&books, processor);

    let stats = tokio::time::timeout(Duration::from_secs(10), applier.next())
        .await
        .expect("timed out waiting for WS book message")
        .expect("WS stream ended unexpectedly")
        .expect("WS processing error");

    assert!(
        stats.book_messages > 0,
        "expected at least one book message"
    );

    let snapshot = books.get_book(&token_id).expect("failed to read book");
    assert!(
        !snapshot.bids.is_empty() || !snapshot.asks.is_empty(),
        "expected some book levels after applying an update"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_ws_market_book_applier_connection_stable() {
    // Subscribe to a handful of active tokens and keep the connection open for a while.
    // We don't require constant message flow; we just fail on close or error.
    let client = ClobClient::new(HOST);
    let markets = client
        .get_sampling_markets(None)
        .await
        .expect("failed to fetch markets");

    let token_ids: Vec<String> = markets
        .data
        .iter()
        .filter(|m| m.active && !m.closed)
        .filter_map(|m| m.tokens.first().map(|t| t.token_id.clone()))
        .take(10)
        .collect();

    assert!(!token_ids.is_empty(), "no active token IDs found");

    let books = OrderBookManager::new(256);
    for token_id in &token_ids {
        books
            .get_or_create_book(token_id)
            .expect("failed to create book");
    }

    let mut ws = WebSocketStream::new(WS_MARKET_URL);
    ws.subscribe_market_channel(token_ids.clone())
        .await
        .expect("failed to subscribe market channel");

    let processor = WsBookUpdateProcessor::new(256 * 1024);
    let mut applier = ws.into_book_applier(&books, processor);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(stability_secs(15));
    let mut saw_book_message = false;

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), applier.next()).await {
            Ok(Some(Ok(stats))) => {
                if stats.book_messages > 0 {
                    saw_book_message = true;
                }
            },
            Ok(Some(Err(e))) => panic!("WS processing error: {:?}", e),
            Ok(None) => panic!("WS stream ended unexpectedly"),
            Err(_) => {
                // No message in this interval; keep waiting.
            },
        }
    }

    assert!(
        saw_book_message,
        "did not observe any `book` messages during stability window"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_ws_user_channel_connection_stable() {
    // Connect + subscribe to the user channel and keep the connection open for a while.
    //
    // Note: depending on account activity, the WS may not emit any `order`/`trade` messages
    // during the window. This test primarily asserts we can authenticate + subscribe
    // and that the connection doesn't immediately drop.
    let private_key = load_private_key();
    let auth_client = ClobClient::with_l1_headers(HOST, &private_key, CHAIN_ID);
    let api_creds = auth_client
        .create_or_derive_api_key(None)
        .await
        .expect("failed to create/derive api key");

    let markets = auth_client
        .get_sampling_markets(None)
        .await
        .expect("failed to fetch markets");
    let market_id = markets
        .data
        .iter()
        .find(|m| m.active && !m.closed)
        .map(|m| m.condition_id.clone())
        .expect("no active markets found");

    let mut ws = WebSocketStream::new(WS_USER_URL).with_auth(api_creds);
    ws.subscribe_user_channel(vec![market_id])
        .await
        .expect("failed to subscribe user channel");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(stability_secs(10));
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), ws.next()).await {
            Ok(Some(Ok(_msg))) => {},
            Ok(Some(Err(e))) => panic!("WS error: {:?}", e),
            Ok(None) => panic!("WS stream ended unexpectedly"),
            Err(_) => {
                // No message in this interval; keep waiting.
            },
        }
    }
}
