//! Fee estimation for Polymarket V2 CLOB markets.
//!
//! Polymarket applies fees at match time. Signed orders do not include fee information; callers
//! should query `ClobClient::get_clob_market_info(...).fee_details` for market fee parameters and
//! use this module only for estimation or simulation.

use crate::types::ClobFeeDetails;
use rust_decimal::Decimal;

/// Estimate the taker fee for a V2 CLOB order.
///
/// Polymarket applies fees at match time; this function is for local estimation and simulation
/// only. Fee parameters come from `ClobClient::get_clob_market_info(condition_id).fee_details`.
///
/// # Arguments
/// * `amount_quote` - Quote amount for the order.
/// * `price` - Price per share.
/// * `fee_details` - V2 CLOB market fee parameters.
///
/// # Returns
/// Estimated fee amount in quote currency.
pub fn calculate_taker_fee(
    amount_quote: Decimal,
    price: Decimal,
    fee_details: &ClobFeeDetails,
) -> Decimal {
    let base = price * (Decimal::ONE - price);
    let base_power = (0..fee_details.fee_exponent).fold(Decimal::ONE, |acc, _| acc * base);
    let fee_rate = Decimal::from(fee_details.fee_rate) / Decimal::from(10_000);

    amount_quote * fee_rate * base_power / price
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    struct FeeCase {
        price: Decimal,
        fee_rate: u32,
        fee_exponent: u32,
        expected: Decimal,
    }

    #[test]
    fn test_v2_taker_fee_golden_vectors() {
        let cases = [
            FeeCase {
                price: dec!(0.05),
                fee_rate: 1,
                fee_exponent: 0,
                expected: dec!(0.2),
            },
            FeeCase {
                price: dec!(0.05),
                fee_rate: 100,
                fee_exponent: 1,
                expected: dec!(0.95),
            },
            FeeCase {
                price: dec!(0.5),
                fee_rate: 1,
                fee_exponent: 2,
                expected: dec!(0.00125),
            },
            FeeCase {
                price: dec!(0.5),
                fee_rate: 100,
                fee_exponent: 0,
                expected: dec!(2),
            },
            FeeCase {
                price: dec!(0.95),
                fee_rate: 1,
                fee_exponent: 1,
                expected: dec!(0.0005),
            },
            FeeCase {
                price: dec!(0.95),
                fee_rate: 100,
                fee_exponent: 2,
                expected: dec!(0.002375),
            },
        ];

        for case in cases {
            let fee_details = ClobFeeDetails {
                fee_rate: case.fee_rate,
                fee_exponent: case.fee_exponent,
                taker_only: false,
            };
            assert_eq!(
                calculate_taker_fee(dec!(100), case.price, &fee_details),
                case.expected
            );
        }
    }
}
