// WebSocket integration tests for polyfill-rs
//
// These tests connect to Polymarket's live WS endpoints and are ignored by default.
//
// Run with:
//   cargo test --all-features --test ws_integration_tests -- --ignored --nocapture --test-threads=1

#![cfg(feature = "stream")]

use futures::StreamExt;
use polyfill_rs::{ClobClient, OrderBookManager, WebSocketStream, WsBookUpdateProcessor};
use std::time::Duration;

const HOST: &str = "https://clob.polymarket.com";
const WS_MARKET_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

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
