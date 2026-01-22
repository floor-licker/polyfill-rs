//! Fee calculation for Polymarket 15-minute markets
//!
//! This module provides local fee calculation to avoid API latency.
//! Fee formula: `fee = shares * price * 0.25 * (price * (1 - price))^2`

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Fee rate for maker orders (always 0 - makers get rebates)
pub const FEE_RATE_BPS_MAKER: u32 = 0;

/// Calculate taker fee_rate_bps from price for 15-minute markets.
///
/// Formula: `fee_rate_bps = 2500 * (price * (1 - price))^2`
///
/// This is derived from the fee formula by expressing fee as a percentage of notional:
/// - fee = shares * price * 0.25 * (price * (1 - price))^2
/// - notional = shares * price
/// - fee_rate = fee / notional = 0.25 * (price * (1 - price))^2
/// - fee_rate_bps = fee_rate * 10000 = 2500 * (price * (1 - price))^2
///
/// # Examples
/// - price 0.50: fee_rate_bps = 156 (1.56%)
/// - price 0.10 or 0.90: fee_rate_bps ~20 (0.20%)
/// - price 0.01 or 0.99: fee_rate_bps ~0
pub fn calculate_fee_rate_bps(price: Decimal) -> u32 {
    // fee_rate_bps = 2500 * (price * (1 - price))^2
    let one_minus_price = Decimal::ONE - price;
    let product = price * one_minus_price;
    let squared = product * product;
    let fee_rate_bps = dec!(2500) * squared;

    // Round to nearest integer
    fee_rate_bps.round().try_into().unwrap_or(0)
}

/// Calculate Polymarket taker fee amount for 15-minute markets.
///
/// Formula: `fee = shares * price * 0.25 * (price * (1 - price))^2`
///
/// # Arguments
/// * `shares` - Number of shares in the order
/// * `price` - Price per share (0.0 to 1.0)
///
/// # Returns
/// Fee amount in USDC
pub fn calculate_taker_fee(shares: Decimal, price: Decimal) -> Decimal {
    // fee = shares * price * 0.25 * (price * (1 - price))^2
    let one_minus_price = Decimal::ONE - price;
    let product = price * one_minus_price;
    let squared = product * product;

    shares * price * dec!(0.25) * squared
}

/// Calculate effective fee rate as a percentage for a given price.
///
/// This is the fee as a percentage of the notional value (shares * price).
///
/// # Returns
/// Fee rate as a percentage (e.g., 1.5625 for 1.5625%)
pub fn effective_fee_rate(price: Decimal) -> Decimal {
    // fee_rate_pct = 0.25 * (price * (1 - price))^2 * 100
    let one_minus_price = Decimal::ONE - price;
    let product = price * one_minus_price;
    let squared = product * product;

    dec!(25) * squared // 0.25 * 100 = 25
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_fee_rate_bps_at_50_percent() {
        // At price 0.50: (0.5 * 0.5)^2 = 0.0625
        // fee_rate_bps = 2500 * 0.0625 = 156.25 -> 156
        let fee_rate = calculate_fee_rate_bps(dec!(0.50));
        assert_eq!(fee_rate, 156);
    }

    #[test]
    fn test_fee_rate_bps_at_10_percent() {
        // At price 0.10: (0.1 * 0.9)^2 = 0.0081
        // fee_rate_bps = 2500 * 0.0081 = 20.25 -> 20
        let fee_rate = calculate_fee_rate_bps(dec!(0.10));
        assert_eq!(fee_rate, 20);
    }

    #[test]
    fn test_fee_rate_bps_at_90_percent() {
        // Symmetric with 10%
        let fee_rate = calculate_fee_rate_bps(dec!(0.90));
        assert_eq!(fee_rate, 20);
    }

    #[test]
    fn test_fee_rate_bps_at_extremes() {
        // At price 0.01: (0.01 * 0.99)^2 = 0.00009801
        // fee_rate_bps = 2500 * 0.00009801 = 0.245 -> 0
        let fee_rate_low = calculate_fee_rate_bps(dec!(0.01));
        assert!(fee_rate_low <= 1);

        let fee_rate_high = calculate_fee_rate_bps(dec!(0.99));
        assert!(fee_rate_high <= 1);
    }

    #[test]
    fn test_taker_fee_calculation() {
        // 100 shares at 0.50 price
        // fee = 100 * 0.50 * 0.25 * (0.50 * 0.50)^2
        // fee = 50 * 0.25 * 0.0625 = 0.78125
        let fee = calculate_taker_fee(dec!(100), dec!(0.50));
        assert_eq!(fee, dec!(0.78125));
    }

    #[test]
    fn test_effective_fee_rate_at_50_percent() {
        // At price 0.50: 25 * (0.25)^2 = 25 * 0.0625 = 1.5625%
        let rate = effective_fee_rate(dec!(0.50));
        assert_eq!(rate, dec!(1.5625));
    }

    #[test]
    fn test_effective_fee_rate_symmetry() {
        // Fee rate should be symmetric around 0.50
        let rate_10 = effective_fee_rate(dec!(0.10));
        let rate_90 = effective_fee_rate(dec!(0.90));
        assert_eq!(rate_10, rate_90);

        let rate_30 = effective_fee_rate(dec!(0.30));
        let rate_70 = effective_fee_rate(dec!(0.70));
        assert_eq!(rate_30, rate_70);
    }

    #[test]
    fn test_maker_fee_rate_constant() {
        assert_eq!(FEE_RATE_BPS_MAKER, 0);
    }
}
