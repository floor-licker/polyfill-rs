// Real API integration tests for /prices-history.
//
// These tests hit Polymarket's live HTTP API and are ignored by default.
//
// Run with:
//   cargo test --all-features --test prices_history_integration_tests -- --ignored --nocapture --test-threads=1

use polyfill_rs::{ClobClient, PricesHistoryInterval};

const HOST: &str = "https://clob.polymarket.com";

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_real_api_get_prices_history_interval_parses() {
    let client = ClobClient::new(HOST);
    let markets = client
        .get_sampling_markets(None)
        .await
        .expect("failed to fetch sampling markets");

    let token_ids: Vec<String> = markets
        .data
        .iter()
        .filter(|m| m.active && !m.closed)
        .filter_map(|m| m.tokens.first().map(|t| t.token_id.clone()))
        .take(20)
        .collect();

    assert!(!token_ids.is_empty(), "no active token IDs found");

    // The API sometimes returns empty history for some markets; try a few and
    // assert at least one returns a non-empty series (verifies endpoint semantics).
    for token_id in token_ids {
        let response = client
            .get_prices_history_interval(&token_id, PricesHistoryInterval::OneDay, Some(5))
            .await
            .expect("failed to fetch prices history");

        if !response.history.is_empty() {
            return;
        }
    }

    panic!("expected at least one active market to return non-empty price history");
}

