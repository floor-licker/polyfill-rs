//! Fee calculation verification and order submission test
//!
//! This example:
//! 1. Verifies local fee calculation matches API across multiple 15-minute markets
//! 2. Tests maker and taker order submission on a test market
//!
//! Required environment variables:
//! - PRIVATE_KEY: Ethereum private key for signing orders
//! - POLYMARKET_API_KEY: API key (optional, will be derived if not set)
//! - POLYMARKET_SECRET: API secret (optional, will be derived if not set)
//! - POLYMARKET_PASSPHRASE: API passphrase (optional, will be derived if not set)

use polyfill_rs::{
    calculate_fee_rate_bps, ClobClient, OrderArgs, OrderType, Result, Side, FEE_RATE_BPS_MAKER,
};
use rust_decimal_macros::dec;
use tracing::{error, info, warn};

// Greenland market YES token for live order testing
const GREENLAND_YES_TOKEN: &str =
    "5161623255678193352839985156330393796378434470119114669671615782853260939535";

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Polymarket Fee Calculation Test");
    info!("================================");

    // Load environment variables
    dotenv::dotenv().ok();

    // Create basic client for API verification
    let client = ClobClient::new("https://clob.polymarket.com");

    // Part A: Verify fee calculation against API
    info!("\n[Part A] Fee Calculation Verification");
    info!("--------------------------------------");
    verify_fee_calculations(&client).await?;

    // Part B: Test live order submission (requires credentials)
    info!("\n[Part B] Live Order Submission Test");
    info!("------------------------------------");

    match setup_authenticated_client().await {
        Ok(auth_client) => {
            test_order_submission(&auth_client).await?;
        },
        Err(e) => {
            warn!("Skipping order submission test: {}", e);
            warn!("Set PRIVATE_KEY environment variable to enable order testing");
        },
    }

    info!("\nFee test completed successfully!");
    Ok(())
}

/// Verify local fee calculation matches API for various prices
async fn verify_fee_calculations(client: &ClobClient) -> Result<()> {
    info!("Fetching 15-minute markets for fee verification...");

    // Get sampling markets and filter for 15-minute crypto markets
    let markets = client.get_sampling_markets(None).await?;

    let mut verified_count = 0;
    let mut mismatches = Vec::new();

    for market in markets.data.iter() {
        // Check if this is a 15-minute crypto market
        let q = market.question.to_lowercase();
        let is_15min = q.contains("bitcoin")
            && q.contains("up or down")
            && ((q.contains(":00") && q.contains(":15"))
                || (q.contains(":15") && q.contains(":30"))
                || (q.contains(":30") && q.contains(":45"))
                || (q.contains(":45") && q.contains(":00")));

        if !market.active || market.closed || !is_15min {
            continue;
        }

        // Get token price and fee rate from API for each token
        for token in &market.tokens {
            if token.token_id.is_empty() {
                continue;
            }

            let price = token.price;
            if price <= dec!(0) || price >= dec!(1) {
                continue;
            }

            // Get fee rate from API
            match client.get_fee_rate(&token.token_id).await {
                Ok(api_fee_rate) => {
                    // Calculate local fee rate
                    let local_fee_rate = calculate_fee_rate_bps(price);

                    // Compare (allow small rounding differences)
                    let diff = (api_fee_rate as i32 - local_fee_rate as i32).abs();

                    if diff <= 1 {
                        verified_count += 1;
                        info!(
                            "  {} ({}): price={:.4}, local={}, api={} ",
                            token.outcome,
                            &token.token_id[..20],
                            price,
                            local_fee_rate,
                            api_fee_rate
                        );
                    } else {
                        mismatches.push((
                            market.question.clone(),
                            token.outcome.clone(),
                            price,
                            local_fee_rate,
                            api_fee_rate,
                        ));
                    }
                },
                Err(e) => {
                    warn!("  Failed to get fee rate for {}: {}", token.token_id, e);
                },
            }

            // Limit to avoid too many API calls
            if verified_count >= 10 {
                break;
            }
        }

        if verified_count >= 10 {
            break;
        }
    }

    info!("\nVerification Summary:");
    info!("  Matched: {} tokens", verified_count);
    info!("  Mismatches: {} tokens", mismatches.len());

    if !mismatches.is_empty() {
        error!("\nMismatched calculations:");
        for (question, outcome, price, local, api) in &mismatches {
            error!(
                "  {} ({}): price={:.4}, local={}, api={}",
                question, outcome, price, local, api
            );
        }
        return Err(polyfill_rs::PolyfillError::validation(
            "Fee calculation mismatch detected",
        ));
    }

    if verified_count == 0 {
        warn!("No 15-minute markets found for verification");
        warn!("Testing formula at known price points instead...");

        // Test at known price points
        let test_cases = vec![
            (dec!(0.50), 156u32), // Max fee at 50%
            (dec!(0.10), 20u32),  // Low price
            (dec!(0.90), 20u32),  // High price (symmetric)
            (dec!(0.25), 88u32),  // Mid-low
            (dec!(0.75), 88u32),  // Mid-high (symmetric)
        ];

        for (price, expected_approx) in test_cases {
            let calculated = calculate_fee_rate_bps(price);
            let diff = (calculated as i32 - expected_approx as i32).abs();
            if diff <= 2 {
                info!(
                    "  price={:.2}: calculated={} bps (expected ~{})",
                    price, calculated, expected_approx
                );
            } else {
                error!(
                    "  price={:.2}: calculated={} bps, expected ~{}",
                    price, calculated, expected_approx
                );
            }
        }
    }

    info!("Fee calculation verification complete!");
    Ok(())
}

/// Set up authenticated client for order submission
async fn setup_authenticated_client() -> std::result::Result<ClobClient, String> {
    let private_key = std::env::var("PRIVATE_KEY").map_err(|_| "PRIVATE_KEY not set")?;

    // Create client with L1 headers for order signing
    let mut client = ClobClient::with_l1_headers(
        "https://clob.polymarket.com",
        &private_key,
        137, // Polygon mainnet
    );

    // Print wallet address
    if let Some(addr) = client.get_address() {
        info!("EOA (signer) address: {}", addr);
    }
    if let Some(proxy) = client.derive_proxy_address() {
        info!("Derived proxy address: {}", proxy);
    }

    // Set the actual funder/proxy address from Polymarket UI
    // sig_type: 1 = PolyProxy, 2 = PolyGnosisSafe
    let funder = std::env::var("POLYMARKET_FUNDER")
        .unwrap_or_else(|_| "0x2884bBb0F04ADca41e7F21A9b18CE43345223E06".to_string());
    let sig_type: u8 = std::env::var("POLYMARKET_SIG_TYPE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2); // Default to PolyGnosisSafe

    info!("Setting funder address: {} (sig_type={})", funder, sig_type);
    client.set_funder(&funder, sig_type);

    // Check if we have API credentials or need to derive them
    if let (Ok(api_key), Ok(secret), Ok(passphrase)) = (
        std::env::var("POLYMARKET_API_KEY"),
        std::env::var("POLYMARKET_SECRET"),
        std::env::var("POLYMARKET_PASSPHRASE"),
    ) {
        let api_creds = polyfill_rs::ApiCredentials {
            api_key,
            secret,
            passphrase,
        };
        client.set_api_creds(api_creds);
        info!("Using provided API credentials");
    } else {
        // Derive API credentials
        info!("Deriving API credentials...");
        let api_creds = client
            .create_or_derive_api_key(None)
            .await
            .map_err(|e| format!("Failed to derive API key: {}", e))?;
        client.set_api_creds(api_creds);
        info!("API credentials derived successfully");
    }

    Ok(client)
}

/// Test maker and taker order submission
async fn test_order_submission(client: &ClobClient) -> Result<()> {
    info!("Testing order submission on Greenland market...");

    // Get order book to find current prices
    let order_book = client.get_order_book(GREENLAND_YES_TOKEN).await?;

    if order_book.asks.is_empty() {
        warn!("No asks in order book, skipping taker order test");
        return Ok(());
    }

    let best_ask = &order_book.asks[0];
    let best_bid = order_book.bids.first();

    info!(
        "  Order book: best_ask={}, best_bid={:?}",
        best_ask.price,
        best_bid.map(|b| b.price)
    );

    // Get tick size
    let tick_size = client.get_tick_size(GREENLAND_YES_TOKEN).await?;
    info!("  Tick size: {}", tick_size);

    // Minimum order size is 5 shares on Polymarket
    let min_size = dec!(5);

    // Test 1: Maker order (passive, resting on book)
    info!("\n  [Test 1] Maker Order (fee_rate_bps=0)");

    // Place order well below best ask to ensure it rests
    let maker_price = if let Some(bid) = best_bid {
        bid.price
    } else {
        best_ask.price - tick_size * dec!(10)
    };

    let maker_order_args = OrderArgs::new(GREENLAND_YES_TOKEN, maker_price, min_size, Side::BUY);

    // Create order with fee_rate_bps = 0 (maker)
    let maker_extras = polyfill_rs::types::ExtraOrderArgs {
        fee_rate_bps: FEE_RATE_BPS_MAKER,
        ..Default::default()
    };

    match client
        .create_order(&maker_order_args, None, Some(maker_extras), None)
        .await
    {
        Ok(signed_order) => {
            info!("    Created maker order: salt={}", signed_order.salt);
            info!(
                "    fee_rate_bps in signed order: {}",
                signed_order.fee_rate_bps
            );

            match client.post_order(signed_order, OrderType::GTC).await {
                Ok(result) => {
                    if let Some(order_id) = result.get("orderID").and_then(|v| v.as_str()) {
                        info!("    Maker order posted: {}", order_id);

                        // Cancel the order immediately
                        match client.cancel(order_id).await {
                            Ok(_) => info!("    Maker order cancelled"),
                            Err(e) => warn!("    Failed to cancel: {}", e),
                        }
                    } else {
                        info!("    Maker order result: {:?}", result);
                    }
                },
                Err(e) => {
                    error!("    Failed to post maker order: {}", e);
                    return Err(e);
                },
            }
        },
        Err(e) => {
            error!("    Failed to create maker order: {}", e);
            return Err(e);
        },
    }

    // Test 2: Taker order (crossing, taking liquidity)
    info!("\n  [Test 2] Taker Order (calculated fee_rate_bps)");

    let taker_price = best_ask.price;
    let taker_fee_rate = calculate_fee_rate_bps(taker_price);
    info!(
        "    Price: {}, calculated fee_rate_bps: {}",
        taker_price, taker_fee_rate
    );

    let taker_order_args = OrderArgs::new(GREENLAND_YES_TOKEN, taker_price, min_size, Side::BUY);

    // Create order with calculated fee_rate_bps (taker)
    let taker_extras = polyfill_rs::types::ExtraOrderArgs {
        fee_rate_bps: taker_fee_rate,
        ..Default::default()
    };

    match client
        .create_order(&taker_order_args, None, Some(taker_extras), None)
        .await
    {
        Ok(signed_order) => {
            info!("    Created taker order: salt={}", signed_order.salt);
            info!(
                "    fee_rate_bps in signed order: {}",
                signed_order.fee_rate_bps
            );

            // For taker orders, use FOK (Fill-or-Kill) to ensure immediate execution
            match client.post_order(signed_order, OrderType::FOK).await {
                Ok(result) => {
                    if let Some(order_id) = result.get("orderID").and_then(|v| v.as_str()) {
                        let status = result
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        info!("    Taker order result: id={}, status={}", order_id, status);
                    } else {
                        info!("    Taker order result: {:?}", result);
                    }
                },
                Err(e) => {
                    // FOK orders may be rejected if liquidity insufficient
                    warn!("    Taker order not executed: {}", e);
                    warn!("    This may be expected if liquidity is insufficient for FOK");
                },
            }
        },
        Err(e) => {
            error!("    Failed to create taker order: {}", e);
            return Err(e);
        },
    }

    info!("\nOrder submission tests completed!");
    Ok(())
}
