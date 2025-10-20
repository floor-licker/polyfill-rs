//! Authentication and cryptographic utilities for Polymarket API
//!
//! This module provides EIP-712 signing, HMAC authentication, and header generation
//! for secure communication with the Polymarket CLOB API.

use crate::errors::{PolyfillError, Result};
use crate::types::ApiCredentials;
use alloy_primitives::{hex::encode_prefixed, Address, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol};
use base64::engine::Engine;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// Header constants
const POLY_ADDR_HEADER: &str = "poly_address";
const POLY_SIG_HEADER: &str = "poly_signature";
const POLY_TS_HEADER: &str = "poly_timestamp";
const POLY_NONCE_HEADER: &str = "poly_nonce";
const POLY_API_KEY_HEADER: &str = "poly_api_key";
const POLY_PASS_HEADER: &str = "poly_passphrase";

type Headers = HashMap<&'static str, String>;

/// EIP-712 struct for CLOB authentication
sol! {
    struct ClobAuth {
        address address;
        string timestamp;
        uint256 nonce;
        string message;
    }
}

/// EIP-712 struct for order signing
sol! {
    struct Order {
        uint256 salt;
        address maker;
        address signer;
        address taker;
        uint256 tokenId;
        uint256 makerAmount;
        uint256 takerAmount;
        uint256 expiration;
        uint256 nonce;
        uint256 feeRateBps;
        uint8 side;
        uint8 signatureType;
    }
}


/// Get current Unix timestamp in seconds
pub fn get_current_unix_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

/// Sign CLOB authentication message using EIP-712
pub fn sign_clob_auth_message(
    signer: &PrivateKeySigner,
    timestamp: String,
    nonce: U256,
) -> Result<String> {
    let message = "This message attests that I control the given wallet".to_string();
    let polygon = 137;

    let auth_struct = ClobAuth {
        address: signer.address(),
        timestamp,
        nonce,
        message,
    };

    let domain = eip712_domain!(
        name: "ClobAuthDomain",
        version: "1",
        chain_id: polygon,
    );

    let signature = signer
        .sign_typed_data_sync(&auth_struct, &domain)
        .map_err(|e| PolyfillError::crypto(format!("EIP-712 signature failed: {}", e)))?;

    Ok(encode_prefixed(signature.as_bytes()))
}

/// Sign order message using EIP-712
pub fn sign_order_message(
    signer: &PrivateKeySigner,
    order: Order,
    chain_id: u64,
    verifying_contract: Address,
) -> Result<String> {
    let domain = eip712_domain!(
        name: "Polymarket CTF Exchange",
        version: "1",
        chain_id: chain_id,
        verifying_contract: verifying_contract,
    );

    let signature = signer
        .sign_typed_data_sync(&order, &domain)
        .map_err(|e| PolyfillError::crypto(format!("Order signature failed: {}", e)))?;

    Ok(encode_prefixed(signature.as_bytes()))
}

/// Build HMAC signature for L2 authentication
pub fn build_hmac_signature<T>(
    secret: &str,
    timestamp: u64,
    method: &str,
    request_path: &str,
    body: Option<&T>,
) -> Result<String>
where
    T: ?Sized + Serialize,
{
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|e| PolyfillError::crypto(format!("Invalid HMAC key: {}", e)))?;

    // Build the message to sign: timestamp + method + path + body
    let message = format!(
        "{}{}{}{}",
        timestamp,
        method.to_uppercase(),
        request_path,
        match body {
            Some(b) => serde_json::to_string(b)
                .map_err(|e| PolyfillError::parse(format!("Failed to serialize body: {}", e), None))?,
            None => String::new(),
        }
    );

    mac.update(message.as_bytes());
    let result = mac.finalize();
    Ok(base64::engine::general_purpose::STANDARD.encode(result.into_bytes()))
}

/// Create L1 headers for authentication (using private key signature)
pub fn create_l1_headers(signer: &PrivateKeySigner, nonce: Option<U256>) -> Result<Headers> {
    let timestamp = get_current_unix_time_secs().to_string();
    let nonce = nonce.unwrap_or(U256::ZERO);
    let signature = sign_clob_auth_message(signer, timestamp.clone(), nonce)?;
    let address = encode_prefixed(signer.address().as_slice());

    Ok(HashMap::from([
        (POLY_ADDR_HEADER, address),
        (POLY_SIG_HEADER, signature),
        (POLY_TS_HEADER, timestamp),
        (POLY_NONCE_HEADER, nonce.to_string()),
    ]))
}

/// Create L2 headers for API calls (using API key and HMAC)
pub fn create_l2_headers<T>(
    signer: &PrivateKeySigner,
    api_creds: &ApiCredentials,
    method: &str,
    req_path: &str,
    body: Option<&T>,
) -> Result<Headers>
where
    T: ?Sized + Serialize,
{
    let address = encode_prefixed(signer.address().as_slice());
    let timestamp = get_current_unix_time_secs();

    let hmac_signature = build_hmac_signature(&api_creds.secret, timestamp, method, req_path, body)?;

    Ok(HashMap::from([
        (POLY_ADDR_HEADER, address),
        (POLY_SIG_HEADER, hmac_signature),
        (POLY_TS_HEADER, timestamp.to_string()),
        (POLY_API_KEY_HEADER, api_creds.api_key.clone()),
        (POLY_PASS_HEADER, api_creds.passphrase.clone()),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_timestamp() {
        let timestamp = get_current_unix_time_secs();
        assert!(timestamp > 1_600_000_000); // Should be after 2020
    }

    #[test]
    fn test_hmac_signature() {
        let result = build_hmac_signature::<String>(
            "test_secret",
            1234567890,
            "GET",
            "/test",
            None,
        );
        assert!(result.is_ok());
    }
}
