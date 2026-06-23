//! Authentication and cryptographic utilities for Polymarket API
//!
//! This module provides EIP-712 signing, HMAC authentication, and header generation
//! for secure communication with the Polymarket CLOB API.

use crate::errors::{PolyfillError, Result};
use crate::types::ApiCredentials;
use alloy_primitives::{hex::encode_prefixed, Address, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol, Eip712Domain};
use base64::engine::Engine;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// Header constants
const POLY_ADDR_HEADER: &str = "poly_address";
const POLY_SIG_HEADER: &str = "poly_signature";
const POLY_TS_HEADER: &str = "poly_timestamp";
const POLY_NONCE_HEADER: &str = "poly_nonce";
const POLY_API_KEY_HEADER: &str = "poly_api_key";
const POLY_PASS_HEADER: &str = "poly_passphrase";

type Headers = HashMap<&'static str, String>;

pub trait HmacApiCredentials {
    fn api_key(&self) -> &str;
    fn passphrase(&self) -> &str;
    fn decoded_secret_bytes(&self) -> Result<Cow<'_, [u8]>>;
}

#[derive(Debug, Clone)]
pub struct PreparedApiCredentials {
    credentials: ApiCredentials,
    decoded_secret: Arc<[u8]>,
}

impl PreparedApiCredentials {
    pub fn try_new(credentials: ApiCredentials) -> Result<Self> {
        let decoded_secret = base64::engine::general_purpose::URL_SAFE
            .decode(&credentials.secret)
            .map(Into::into)
            .map_err(|e| PolyfillError::crypto(format!("Failed to decode base64 secret: {e}")))?;

        Ok(Self {
            credentials,
            decoded_secret,
        })
    }

    pub fn credentials(&self) -> &ApiCredentials {
        &self.credentials
    }
}

impl Deref for PreparedApiCredentials {
    type Target = ApiCredentials;

    fn deref(&self) -> &Self::Target {
        &self.credentials
    }
}

impl HmacApiCredentials for ApiCredentials {
    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn passphrase(&self) -> &str {
        &self.passphrase
    }

    fn decoded_secret_bytes(&self) -> Result<Cow<'_, [u8]>> {
        Ok(Cow::Owned(decode_secret_bytes(&self.secret)?))
    }
}

impl HmacApiCredentials for PreparedApiCredentials {
    fn api_key(&self) -> &str {
        &self.credentials.api_key
    }

    fn passphrase(&self) -> &str {
        &self.credentials.passphrase
    }

    fn decoded_secret_bytes(&self) -> Result<Cow<'_, [u8]>> {
        Ok(Cow::Borrowed(self.decoded_secret.as_ref()))
    }
}

// EIP-712 struct for CLOB authentication
sol! {
    struct ClobAuth {
        address address;
        string timestamp;
        uint256 nonce;
        string message;
    }
}

// EIP-712 struct for order signing
sol! {
    struct Order {
        uint256 salt;
        address maker;
        address signer;
        uint256 tokenId;
        uint256 makerAmount;
        uint256 takerAmount;
        uint8 side;
        uint8 signatureType;
        uint256 timestamp;
        bytes32 metadata;
        bytes32 builder;
    }
}

/// V2 order signing payload. The REST body still carries `expiration`, but the EIP-712 payload
/// follows the V2 exchange struct.
#[derive(Clone)]
pub struct SignedOrderMessage {
    pub salt: U256,
    pub maker: Address,
    pub signer: Address,
    pub token_id: U256,
    pub maker_amount: U256,
    pub taker_amount: U256,
    pub side: u8,
    pub signature_type: u8,
    pub timestamp: U256,
    pub metadata: B256,
    pub builder: B256,
}

/// Prepared EIP-712 domain for signing orders against one exchange contract.
#[derive(Clone)]
pub struct PreparedOrderDomain {
    domain: Eip712Domain,
}

impl PreparedOrderDomain {
    pub fn new(chain_id: u64, verifying_contract: Address) -> Self {
        let domain = eip712_domain!(
            name: "Polymarket CTF Exchange",
            version: "2",
            chain_id: chain_id,
            verifying_contract: verifying_contract,
        );

        Self { domain }
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
    order: SignedOrderMessage,
    chain_id: u64,
    verifying_contract: Address,
) -> Result<String> {
    let domain = PreparedOrderDomain::new(chain_id, verifying_contract);
    sign_order_message_with_domain(signer, order, &domain)
}

/// Sign order message using a prepared EIP-712 domain.
pub fn sign_order_message_with_domain(
    signer: &PrivateKeySigner,
    order: SignedOrderMessage,
    domain: &PreparedOrderDomain,
) -> Result<String> {
    let order = Order {
        salt: order.salt,
        maker: order.maker,
        signer: order.signer,
        tokenId: order.token_id,
        makerAmount: order.maker_amount,
        takerAmount: order.taker_amount,
        side: order.side,
        signatureType: order.signature_type,
        timestamp: order.timestamp,
        metadata: order.metadata,
        builder: order.builder,
    };

    let signature = signer
        .sign_typed_data_sync(&order, &domain.domain)
        .map_err(|e| PolyfillError::crypto(format!("Order signature failed: {}", e)))?;

    Ok(encode_prefixed(signature.as_bytes()))
}

/// Build HMAC signature for L2 authentication
///
/// Performs cryptographic message authentication using SHA-256 with
/// specialized key derivation and encoding schemes for API compliance.
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
    let decoded_secret = decode_secret_bytes(secret)?;
    let body_bytes =
        match body {
            Some(b) => Some(serde_json::to_vec(b).map_err(|e| {
                PolyfillError::parse(format!("Failed to serialize body: {}", e), None)
            })?),
            None => None,
        };

    build_hmac_signature_bytes(
        &decoded_secret,
        timestamp,
        method,
        request_path,
        body_bytes.as_deref(),
    )
}

pub fn build_hmac_signature_bytes(
    decoded_secret: &[u8],
    timestamp: u64,
    method: &str,
    request_path: &str,
    body_bytes: Option<&[u8]>,
) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(decoded_secret)
        .map_err(|e| PolyfillError::crypto(format!("Invalid HMAC key: {}", e)))?;

    let timestamp = timestamp.to_string();
    mac.update(timestamp.as_bytes());
    let method_upper;
    let method_bytes = if method.bytes().all(|b| !b.is_ascii_lowercase()) {
        method.as_bytes()
    } else {
        method_upper = method.to_ascii_uppercase();
        method_upper.as_bytes()
    };
    mac.update(method_bytes);
    mac.update(request_path.as_bytes());
    if let Some(body_bytes) = body_bytes {
        mac.update(body_bytes);
    }

    let result = mac.finalize();

    Ok(base64::engine::general_purpose::URL_SAFE.encode(result.into_bytes()))
}

fn decode_secret_bytes(secret: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE
        .decode(secret)
        .map_err(|e| PolyfillError::crypto(format!("Failed to decode base64 secret: {}", e)))
}

/// Create L1 headers for authentication (using private key signature)
///
/// Generates initial authentication envelope using elliptic curve cryptography
/// for establishing trusted communication channels with the distributed ledger API.
pub fn create_l1_headers(signer: &PrivateKeySigner, nonce: Option<U256>) -> Result<Headers> {
    // Capture temporal context for replay prevention at protocol boundary
    let timestamp = get_current_unix_time_secs().to_string();
    let nonce = nonce.unwrap_or(U256::ZERO);

    // Generate EIP-712 compliant signature for cryptographic proof of authority
    let signature = sign_clob_auth_message(signer, timestamp.clone(), nonce)?;
    let address = encode_prefixed(signer.address().as_slice());

    // Assemble primary authentication header set with identity binding
    Ok(HashMap::from([
        (POLY_ADDR_HEADER, address),
        (POLY_SIG_HEADER, signature),
        (POLY_TS_HEADER, timestamp),
        (POLY_NONCE_HEADER, nonce.to_string()),
    ]))
}

/// Create L2 headers for API calls (using API key and HMAC)
///
/// Assembles authentication header set with computed signature digest
/// to satisfy bilateral verification requirements at the protocol layer.
pub fn create_l2_headers<T>(
    signer: &PrivateKeySigner,
    api_creds: &(impl HmacApiCredentials + ?Sized),
    method: &str,
    req_path: &str,
    body: Option<&T>,
) -> Result<Headers>
where
    T: ?Sized + Serialize,
{
    // Extract identity from signing authority for header binding
    let address = encode_prefixed(signer.address().as_slice());
    let timestamp = get_current_unix_time_secs();

    // Generate cryptographic authenticator using temporal and message context
    let decoded_secret = api_creds.decoded_secret_bytes()?;
    let body_bytes =
        match body {
            Some(b) => Some(serde_json::to_vec(b).map_err(|e| {
                PolyfillError::parse(format!("Failed to serialize body: {}", e), None)
            })?),
            None => None,
        };
    let hmac_signature = build_hmac_signature_bytes(
        &decoded_secret,
        timestamp,
        method,
        req_path,
        body_bytes.as_deref(),
    )?;

    // Construct header map with authentication primitives in canonical order
    Ok(HashMap::from([
        (POLY_ADDR_HEADER, address),
        (POLY_SIG_HEADER, hmac_signature),
        (POLY_TS_HEADER, timestamp.to_string()),
        (POLY_API_KEY_HEADER, api_creds.api_key().to_string()),
        (POLY_PASS_HEADER, api_creds.passphrase().to_string()),
    ]))
}

pub fn create_l2_headers_with_body_bytes(
    signer: &PrivateKeySigner,
    api_creds: &(impl HmacApiCredentials + ?Sized),
    method: &str,
    req_path: &str,
    body_bytes: Option<&[u8]>,
) -> Result<Headers> {
    let address = encode_prefixed(signer.address().as_slice());
    let timestamp = get_current_unix_time_secs();
    let decoded_secret = api_creds.decoded_secret_bytes()?;
    let hmac_signature =
        build_hmac_signature_bytes(&decoded_secret, timestamp, method, req_path, body_bytes)?;

    Ok(HashMap::from([
        (POLY_ADDR_HEADER, address),
        (POLY_SIG_HEADER, hmac_signature),
        (POLY_TS_HEADER, timestamp.to_string()),
        (POLY_API_KEY_HEADER, api_creds.api_key().to_string()),
        (POLY_PASS_HEADER, api_creds.passphrase().to_string()),
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
            "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1",
            1234567890,
            "GET",
            "/test",
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_hmac_signature_with_body() {
        let body = r#"{"test": "data"}"#;
        let result = build_hmac_signature(
            "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1",
            1234567890,
            "POST",
            "/orders",
            Some(body),
        );
        assert!(result.is_ok());
        let signature = result.unwrap();
        assert!(!signature.is_empty());
    }

    #[test]
    fn test_hmac_signature_consistency() {
        let secret = "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1";
        let timestamp = 1234567890;
        let method = "GET";
        let path = "/test";

        let sig1 = build_hmac_signature::<String>(secret, timestamp, method, path, None).unwrap();
        let sig2 = build_hmac_signature::<String>(secret, timestamp, method, path, None).unwrap();

        // Same inputs should produce same signature
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_hmac_signature_bytes_matches_serialized_body() {
        let secret = "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1";
        let timestamp = 1234567890;
        let body = serde_json::json!({"orderID": "abc123"});
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let decoded_secret = decode_secret_bytes(secret).unwrap();

        let object_signature =
            build_hmac_signature(secret, timestamp, "delete", "/order", Some(&body)).unwrap();
        let bytes_signature = build_hmac_signature_bytes(
            &decoded_secret,
            timestamp,
            "DELETE",
            "/order",
            Some(&body_bytes),
        )
        .unwrap();

        assert_eq!(object_signature, bytes_signature);
    }

    #[test]
    fn test_hmac_signature_different_inputs() {
        let secret = "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1";
        let timestamp = 1234567890;

        let sig1 = build_hmac_signature::<String>(secret, timestamp, "GET", "/test", None).unwrap();
        let sig2 =
            build_hmac_signature::<String>(secret, timestamp, "POST", "/test", None).unwrap();
        let sig3 =
            build_hmac_signature::<String>(secret, timestamp, "GET", "/other", None).unwrap();

        // Different inputs should produce different signatures
        assert_ne!(sig1, sig2);
        assert_ne!(sig1, sig3);
        assert_ne!(sig2, sig3);
    }

    #[test]
    fn test_create_l1_headers() {
        use alloy_primitives::U256;
        use alloy_signer_local::PrivateKeySigner;

        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let signer: PrivateKeySigner = private_key.parse().expect("Valid private key");

        let result = create_l1_headers(&signer, Some(U256::from(12345)));
        assert!(result.is_ok());

        let headers = result.unwrap();
        assert!(headers.contains_key("poly_address"));
        assert!(headers.contains_key("poly_signature"));
        assert!(headers.contains_key("poly_timestamp"));
        assert!(headers.contains_key("poly_nonce"));
    }

    #[test]
    fn test_create_l1_headers_different_nonces() {
        use alloy_primitives::U256;
        use alloy_signer_local::PrivateKeySigner;

        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let signer: PrivateKeySigner = private_key.parse().expect("Valid private key");

        let headers_1 = create_l1_headers(&signer, Some(U256::from(12345))).unwrap();
        let headers_2 = create_l1_headers(&signer, Some(U256::from(54321))).unwrap();

        // Different nonces should produce different signatures
        assert_ne!(
            headers_1.get("poly_signature"),
            headers_2.get("poly_signature")
        );

        // But same address
        assert_eq!(headers_1.get("poly_address"), headers_2.get("poly_address"));
    }

    #[test]
    fn test_create_l2_headers() {
        use alloy_signer_local::PrivateKeySigner;

        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let signer: PrivateKeySigner = private_key.parse().expect("Valid private key");

        let api_creds = ApiCredentials {
            api_key: "test_key".to_string(),
            secret: "dGVzdF9zZWNyZXRfa2V5XzEyMzQ1".to_string(),
            passphrase: "test_passphrase".to_string(),
        };

        let result = create_l2_headers::<String>(&signer, &api_creds, "GET", "/test", None);
        assert!(result.is_ok());

        let headers = result.unwrap();
        assert!(headers.contains_key("poly_api_key"));
        assert!(headers.contains_key("poly_signature"));
        assert!(headers.contains_key("poly_timestamp"));
        assert!(headers.contains_key("poly_passphrase"));

        assert_eq!(headers.get("poly_api_key").unwrap(), "test_key");
        assert_eq!(headers.get("poly_passphrase").unwrap(), "test_passphrase");
    }

    #[test]
    fn test_eip712_signature_format() {
        use alloy_primitives::U256;
        use alloy_signer_local::PrivateKeySigner;

        let private_key = "0x1234567890123456789012345678901234567890123456789012345678901234";
        let signer: PrivateKeySigner = private_key.parse().expect("Valid private key");

        // Test that we can create and sign EIP-712 messages
        let result = create_l1_headers(&signer, Some(U256::from(12345)));
        assert!(result.is_ok());

        let headers = result.unwrap();
        let signature = headers.get("poly_signature").unwrap();

        // EIP-712 signatures should be hex strings of specific length
        assert!(signature.starts_with("0x"));
        assert_eq!(signature.len(), 132); // 0x + 130 hex chars = 132 total
    }

    #[test]
    fn test_timestamp_generation() {
        let ts1 = get_current_unix_time_secs();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let ts2 = get_current_unix_time_secs();

        // Timestamps should be increasing
        assert!(ts2 >= ts1);

        // Should be reasonable current time (after 2020, before 2030)
        assert!(ts1 > 1_600_000_000);
        assert!(ts1 < 1_900_000_000);
    }
}
