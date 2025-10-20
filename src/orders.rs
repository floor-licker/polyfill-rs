//! Order creation and signing functionality
//!
//! This module handles the complex process of creating and signing orders
//! for the Polymarket CLOB, including EIP-712 signature generation.

use crate::auth::sign_order_message;
use crate::errors::{PolyfillError, Result};
use crate::client::OrderArgs;
use crate::types::{ExtraOrderArgs, MarketOrderArgs, OrderOptions, SignedOrderRequest, Side};
use alloy_primitives::{Address, U256};
use alloy_signer_local::PrivateKeySigner;
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy::{AwayFromZero, MidpointTowardZero, ToZero};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Signature types for orders
#[derive(Copy, Clone)]
pub enum SigType {
    /// ECDSA EIP712 signatures signed by EOAs
    Eoa = 0,
    /// EIP712 signatures signed by EOAs that own Polymarket Proxy wallets
    PolyProxy = 1,
    /// EIP712 signatures signed by EOAs that own Polymarket Gnosis safes
    PolyGnosisSafe = 2,
}

/// Rounding configuration for different tick sizes
pub struct RoundConfig {
    price: u32,
    size: u32,
    amount: u32,
}

/// Contract configuration
pub struct ContractConfig {
    pub exchange: String,
    pub collateral: String,
    pub conditional_tokens: String,
}


/// Order builder for creating and signing orders
pub struct OrderBuilder {
    signer: PrivateKeySigner,
    sig_type: SigType,
    funder: Address,
}

/// Rounding configurations for different tick sizes
static ROUNDING_CONFIG: LazyLock<HashMap<Decimal, RoundConfig>> = LazyLock::new(|| {
    HashMap::from([
        (
            Decimal::from_str("0.1").unwrap(),
            RoundConfig {
                price: 1,
                size: 2,
                amount: 3,
            },
        ),
        (
            Decimal::from_str("0.01").unwrap(),
            RoundConfig {
                price: 2,
                size: 2,
                amount: 4,
            },
        ),
        (
            Decimal::from_str("0.001").unwrap(),
            RoundConfig {
                price: 3,
                size: 2,
                amount: 5,
            },
        ),
        (
            Decimal::from_str("0.0001").unwrap(),
            RoundConfig {
                price: 4,
                size: 2,
                amount: 6,
            },
        ),
    ])
});

/// Get contract configuration for chain
pub fn get_contract_config(chain_id: u64, neg_risk: bool) -> Option<ContractConfig> {
    match (chain_id, neg_risk) {
        (137, false) => Some(ContractConfig {
            exchange: "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E".to_string(),
            collateral: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
            conditional_tokens: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".to_string(),
        }),
        (137, true) => Some(ContractConfig {
            exchange: "0xC5d563A36AE78145C45a50134d48A1215220f80a".to_string(),
            collateral: "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174".to_string(),
            conditional_tokens: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".to_string(),
        }),
        _ => None,
    }
}

/// Generate a random seed for order salt
fn generate_seed() -> u64 {
    let mut rng = rand::thread_rng();
    let y: f64 = rng.gen();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();
    (timestamp as f64 * y) as u64
}

/// Convert decimal to token units (multiply by 1e6)
fn decimal_to_token_u32(amt: Decimal) -> u32 {
    let mut amt = Decimal::from_scientific("1e6").expect("1e6 is not scientific") * amt;
    if amt.scale() > 0 {
        amt = amt.round_dp_with_strategy(0, MidpointTowardZero);
    }
    amt.try_into().expect("Couldn't round decimal to integer")
}

impl OrderBuilder {
    /// Create a new order builder
    pub fn new(
        signer: PrivateKeySigner,
        sig_type: Option<SigType>,
        funder: Option<Address>,
    ) -> Self {
        let sig_type = sig_type.unwrap_or(SigType::Eoa);
        let funder = funder.unwrap_or(signer.address());

        OrderBuilder {
            signer,
            sig_type,
            funder,
        }
    }

    /// Get signature type as u8
    pub fn get_sig_type(&self) -> u8 {
        self.sig_type as u8
    }

    /// Fix amount rounding according to configuration
    fn fix_amount_rounding(&self, mut amt: Decimal, round_config: &RoundConfig) -> Decimal {
        if amt.scale() > round_config.amount {
            amt = amt.round_dp_with_strategy(round_config.amount + 4, AwayFromZero);
            if amt.scale() > round_config.amount {
                amt = amt.round_dp_with_strategy(round_config.amount, ToZero);
            }
        }
        amt
    }

    /// Get order amounts (maker and taker) for a regular order
    fn get_order_amounts(
        &self,
        side: Side,
        size: Decimal,
        price: Decimal,
        round_config: &RoundConfig,
    ) -> (u32, u32) {
        let raw_price = price.round_dp_with_strategy(round_config.price, MidpointTowardZero);

        match side {
            Side::BUY => {
                let raw_taker_amt = size.round_dp_with_strategy(round_config.size, ToZero);
                let raw_maker_amt = raw_taker_amt * raw_price;
                let raw_maker_amt = self.fix_amount_rounding(raw_maker_amt, round_config);
                (
                    decimal_to_token_u32(raw_maker_amt),
                    decimal_to_token_u32(raw_taker_amt),
                )
            }
            Side::SELL => {
                let raw_maker_amt = size.round_dp_with_strategy(round_config.size, ToZero);
                let raw_taker_amt = raw_maker_amt * raw_price;
                let raw_taker_amt = self.fix_amount_rounding(raw_taker_amt, round_config);

                (
                    decimal_to_token_u32(raw_maker_amt),
                    decimal_to_token_u32(raw_taker_amt),
                )
            }
        }
    }

    /// Get order amounts for a market order
    fn get_market_order_amounts(
        &self,
        amount: Decimal,
        price: Decimal,
        round_config: &RoundConfig,
    ) -> (u32, u32) {
        let raw_maker_amt = amount.round_dp_with_strategy(round_config.size, ToZero);
        let raw_price = price.round_dp_with_strategy(round_config.price, MidpointTowardZero);

        let raw_taker_amt = raw_maker_amt / raw_price;
        let raw_taker_amt = self.fix_amount_rounding(raw_taker_amt, round_config);

        (
            decimal_to_token_u32(raw_maker_amt),
            decimal_to_token_u32(raw_taker_amt),
        )
    }

    /// Calculate market price from order book levels
    pub fn calculate_market_price(
        &self,
        positions: &[crate::types::BookLevel],
        amount_to_match: Decimal,
    ) -> Result<Decimal> {
        let mut sum = Decimal::ZERO;

        for level in positions {
            sum += level.size * level.price;
            if sum >= amount_to_match {
                return Ok(level.price);
            }
        }
        
        Err(PolyfillError::order(
            format!("Not enough liquidity to create market order with amount {}", amount_to_match),
            crate::errors::OrderErrorKind::InsufficientBalance,
        ))
    }

    /// Create a market order
    pub fn create_market_order(
        &self,
        chain_id: u64,
        order_args: &MarketOrderArgs,
        price: Decimal,
        extras: &ExtraOrderArgs,
        options: &OrderOptions,
    ) -> Result<SignedOrderRequest> {
        let tick_size = options.tick_size
            .ok_or_else(|| PolyfillError::validation("Cannot create order without tick size"))?;
        
        let (maker_amount, taker_amount) = self.get_market_order_amounts(
            order_args.amount,
            price,
            &ROUNDING_CONFIG[&tick_size],
        );

        let neg_risk = options.neg_risk
            .ok_or_else(|| PolyfillError::validation("Cannot create order without neg_risk"))?;

        let contract_config = get_contract_config(chain_id, neg_risk)
            .ok_or_else(|| PolyfillError::config("No contract found with given chain_id and neg_risk"))?;

        let exchange_address = Address::from_str(&contract_config.exchange)
            .map_err(|e| PolyfillError::config(format!("Invalid exchange address: {}", e)))?;

        self.build_signed_order(
            order_args.token_id.clone(),
            Side::BUY,
            chain_id,
            exchange_address,
            maker_amount,
            taker_amount,
            0,
            extras,
        )
    }

    /// Create a regular order
    pub fn create_order(
        &self,
        chain_id: u64,
        order_args: &OrderArgs,
        expiration: u64,
        extras: &ExtraOrderArgs,
        options: &OrderOptions,
    ) -> Result<SignedOrderRequest> {
        let tick_size = options.tick_size
            .ok_or_else(|| PolyfillError::validation("Cannot create order without tick size"))?;
        
        let (maker_amount, taker_amount) = self.get_order_amounts(
            order_args.side,
            order_args.size,
            order_args.price,
            &ROUNDING_CONFIG[&tick_size],
        );

        let neg_risk = options.neg_risk
            .ok_or_else(|| PolyfillError::validation("Cannot create order without neg_risk"))?;

        let contract_config = get_contract_config(chain_id, neg_risk)
            .ok_or_else(|| PolyfillError::config("No contract found with given chain_id and neg_risk"))?;

        let exchange_address = Address::from_str(&contract_config.exchange)
            .map_err(|e| PolyfillError::config(format!("Invalid exchange address: {}", e)))?;

        self.build_signed_order(
            order_args.token_id.clone(),
            order_args.side,
            chain_id,
            exchange_address,
            maker_amount,
            taker_amount,
            expiration,
            extras,
        )
    }

    /// Build and sign an order
    #[allow(clippy::too_many_arguments)]
    fn build_signed_order(
        &self,
        token_id: String,
        side: Side,
        chain_id: u64,
        exchange: Address,
        maker_amount: u32,
        taker_amount: u32,
        expiration: u64,
        extras: &ExtraOrderArgs,
    ) -> Result<SignedOrderRequest> {
        let seed = generate_seed();
        let taker_address = Address::from_str(&extras.taker)
            .map_err(|e| PolyfillError::validation(format!("Invalid taker address: {}", e)))?;

        let u256_token_id = U256::from_str_radix(&token_id, 10)
            .map_err(|e| PolyfillError::validation(format!("Incorrect tokenId format: {}", e)))?;

        let order = crate::auth::Order {
            salt: U256::from(seed),
            maker: self.funder,
            signer: self.signer.address(),
            taker: taker_address,
            tokenId: u256_token_id,
            makerAmount: U256::from(maker_amount),
            takerAmount: U256::from(taker_amount),
            expiration: U256::from(expiration),
            nonce: extras.nonce,
            feeRateBps: U256::from(extras.fee_rate_bps),
            side: side as u8,
            signatureType: self.sig_type as u8,
        };

        let signature = sign_order_message(&self.signer, order, chain_id, exchange)?;

        Ok(SignedOrderRequest {
            salt: seed,
            maker: self.funder.to_checksum(None),
            signer: self.signer.address().to_checksum(None),
            taker: taker_address.to_checksum(None),
            token_id,
            maker_amount: maker_amount.to_string(),
            taker_amount: taker_amount.to_string(),
            expiration: expiration.to_string(),
            nonce: extras.nonce.to_string(),
            fee_rate_bps: extras.fee_rate_bps.to_string(),
            side: side.as_str().to_string(),
            signature_type: self.sig_type as u8,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decimal_to_token_u32() {
        let result = decimal_to_token_u32(Decimal::from_str("1.5").unwrap());
        assert_eq!(result, 1_500_000);
    }

    #[test]
    fn test_generate_seed() {
        let seed1 = generate_seed();
        let seed2 = generate_seed();
        assert_ne!(seed1, seed2);
    }
}
