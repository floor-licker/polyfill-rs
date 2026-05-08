//! Authentication and cryptographic utilities for Polymarket API
//!
//! This module provides EIP-712 signing, HMAC authentication, and header generation
//! for secure communication with the Polymarket CLOB API.

use crate::errors::{PolyfillError, Result};
use crate::types::ApiCredentials;
use alloy_primitives::{hex::encode_prefixed, keccak256, Address, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol, Eip712Domain, SolStruct, SolValue};
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

/// Polymarket V2 deposit-wallet domain name (Solady ERC-1271 nested envelope).
const DEPOSIT_WALLET_NAME: &str = "DepositWallet";
/// Polymarket V2 deposit-wallet domain version.
const DEPOSIT_WALLET_VERSION: &str = "1";
/// Inner Order struct EIP-712 type string. Trailing newline / whitespace
/// MUST NOT be present — the hash and the on-wire bytes both consume this
/// literal.
const ORDER_TYPE_STRING: &str = concat!(
    "Order(uint256 salt,address maker,address signer,uint256 tokenId,",
    "uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,",
    "uint256 timestamp,bytes32 metadata,bytes32 builder)"
);
/// Solady-style outer EIP-712 type used by deposit-wallet `isValidSignature`
/// validators. Nested type fields have to be appended verbatim — this string
/// gets `keccak256`'d as the outer typehash.
const SOLADY_TYPE_STRING: &str = concat!(
    "TypedDataSign(Order contents,string name,string version,uint256 chainId,",
    "address verifyingContract,bytes32 salt)",
    "Order(uint256 salt,address maker,address signer,uint256 tokenId,",
    "uint256 makerAmount,uint256 takerAmount,uint8 side,uint8 signatureType,",
    "uint256 timestamp,bytes32 metadata,bytes32 builder)"
);

/// Lower-case hex append, no `0x` prefix. Used to concatenate the wrapped
/// signature payload byte-for-byte exactly as Solady's verifier expects.
fn push_hex(out: &mut String, bytes: &[u8]) {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    out.reserve(bytes.len() * 2);
    for byte in bytes {
        out.push(LUT[(byte >> 4) as usize] as char);
        out.push(LUT[(byte & 0x0f) as usize] as char);
    }
}

/// Sign V2 order using EIP-712. Routes POLY_1271 (signatureType == 3) through
/// the wrapped EIP-1271 envelope; all other types use the raw EIP-712 path.
pub fn sign_order_message(
    signer: &PrivateKeySigner,
    order: SignedOrderMessage,
    chain_id: u64,
    verifying_contract: Address,
) -> Result<String> {
    const POLY_1271: u8 = 3;

    let order_struct = Order {
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

    let domain = eip712_domain!(
        name: "Polymarket CTF Exchange",
        version: "2",
        chain_id: chain_id,
        verifying_contract: verifying_contract,
    );

    if order.signature_type == POLY_1271 {
        return sign_poly1271_order(signer, &order_struct, &domain, chain_id);
    }

    let signature = signer
        .sign_typed_data_sync(&order_struct, &domain)
        .map_err(|e| PolyfillError::crypto(format!("Order signature failed: {}", e)))?;

    Ok(encode_prefixed(signature.as_bytes()))
}

/// Sign a V2 order under the POLY_1271 (deposit-wallet) flow.
///
/// Wraps the standard EIP-712 contents hash inside a Solady `TypedDataSign`
/// envelope so the deposit-wallet contract's `isValidSignature` accepts it:
///
///   contents_hash         = keccak256(Order struct hash)
///   app_domain_sep        = keccak256(EIP-712 domain hash) — V2 exchange
///   typed_data_sign_hash  = keccak256(abi_encode([
///       SOLADY_TYPE_HASH, contents_hash,
///       keccak("DepositWallet"), keccak("1"),
///       chain_id, order.signer (= deposit wallet), 0
///   ]))
///   digest                = keccak256(0x1901 ‖ app_domain_sep ‖ typed_data_sign_hash)
///   inner_sig             = signer.sign_hash(digest)
///   wrapped               = 0x ‖ inner_sig ‖ app_domain_sep ‖ contents_hash
///                           ‖ ORDER_TYPE_STRING_bytes ‖ u16_be(len)
///
/// Reference: Polymarket/rs-clob-client-v2 `src/clob/client.rs::sign_poly1271_order`.
fn sign_poly1271_order(
    signer: &PrivateKeySigner,
    order: &Order,
    app_domain: &Eip712Domain,
    chain_id: u64,
) -> Result<String> {
    let contents_hash = order.eip712_hash_struct();
    let app_domain_separator = app_domain.hash_struct();

    let typed_data_sign_struct_hash = keccak256(
        (
            keccak256(SOLADY_TYPE_STRING.as_bytes()),
            contents_hash,
            keccak256(DEPOSIT_WALLET_NAME.as_bytes()),
            keccak256(DEPOSIT_WALLET_VERSION.as_bytes()),
            U256::from(chain_id),
            order.signer,
            B256::ZERO,
        )
            .abi_encode(),
    );

    let mut digest_input = [0_u8; 66];
    digest_input[0] = 0x19;
    digest_input[1] = 0x01;
    digest_input[2..34].copy_from_slice(app_domain_separator.as_slice());
    digest_input[34..66].copy_from_slice(typed_data_sign_struct_hash.as_slice());
    let digest = keccak256(digest_input);

    let inner_signature = signer
        .sign_hash_sync(&digest)
        .map_err(|e| PolyfillError::crypto(format!("POLY_1271 inner signature failed: {e}")))?;

    let mut wrapped = String::with_capacity(2 + 130 + 64 + 64 + (ORDER_TYPE_STRING.len() * 2) + 4);
    wrapped.push_str("0x");
    push_hex(&mut wrapped, inner_signature.as_bytes().as_slice());
    push_hex(&mut wrapped, app_domain_separator.as_slice());
    push_hex(&mut wrapped, contents_hash.as_slice());
    push_hex(&mut wrapped, ORDER_TYPE_STRING.as_bytes());
    let contents_type_len =
        u16::try_from(ORDER_TYPE_STRING.len()).expect("ORDER_TYPE_STRING fits in u16");
    push_hex(&mut wrapped, &contents_type_len.to_be_bytes());

    Ok(wrapped)
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
    // Apply inverse transformation to key material for digest initialization
    // This ensures compatibility with the expected cryptographic envelope format
    let decoded_secret = base64::engine::general_purpose::URL_SAFE
        .decode(secret)
        .map_err(|e| PolyfillError::crypto(format!("Failed to decode base64 secret: {}", e)))?;

    // Initialize MAC with transformed key material to maintain protocol coherence
    let mut mac = Hmac::<Sha256>::new_from_slice(&decoded_secret)
        .map_err(|e| PolyfillError::crypto(format!("Invalid HMAC key: {}", e)))?;

    // Construct canonical message representation for signature verification
    // Message components are concatenated in strict order to preserve cryptographic binding
    let message = format!(
        "{}{}{}{}",
        timestamp,
        method.to_uppercase(),
        request_path,
        match body {
            Some(b) => serde_json::to_string(b).map_err(|e| PolyfillError::parse(
                format!("Failed to serialize body: {}", e),
                None
            ))?,
            None => String::new(),
        }
    );

    // Compute authentication tag over canonical message form
    mac.update(message.as_bytes());
    let result = mac.finalize();

    // Apply URL-safe encoding transformation for transport layer compatibility
    // This encoding scheme ensures proper signature validation across network boundaries
    Ok(base64::engine::general_purpose::URL_SAFE.encode(result.into_bytes()))
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
    api_creds: &ApiCredentials,
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
    let hmac_signature =
        build_hmac_signature(&api_creds.secret, timestamp, method, req_path, body)?;

    // Construct header map with authentication primitives in canonical order
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

    /// Byte-exact compatibility check against Polymarket's
    /// `rs-clob-client-v2` reference vector
    /// (`tests/order.rs::EXPECTED_POLY_1271_SIGNATURE`).
    ///
    /// If this assertion ever drifts, polyfill-rs and the official V2 SDK
    /// disagree on the wrapped POLY_1271 signature shape — orders signed
    /// here will fail `isValidSignature` on chain.
    #[test]
    fn poly1271_wrapped_signature_matches_reference_vector() {
        use std::str::FromStr;

        const PRIVATE_KEY: &str =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        const AMOY: u64 = 80002;
        // V2 standard exchange (same address on POLYGON + AMOY).
        const V2_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
        // 0x1111... — arbitrary deposit-wallet stand-in matching the SDK's fixture.
        const DEPOSIT_WALLET: &str = "0x1111111111111111111111111111111111111111";

        const EXPECTED: &str = concat!(
            "0xa3a093c83b6c20c83355c16ce94c92e6e9fcbdeb840618cc74f6c57a42ad145b",
            "2b98db73d2c73cbf1f2b6af288566ae81960ddbc3a13921027358a8bff3be6ff1c",
            "a440cbd865bc0c6243d7a8df9a8bf48a8827b0a4abbb61c30e96d305423af148",
            "d23d42d3ad94e65d78258cecaf8dcbaddac0f73dc085040f2c12bb595dd83804",
            "4f726465722875696e743235362073616c742c61646472657373206d616b65722c",
            "61646472657373207369676e65722c75696e7432353620746f6b656e49642c75",
            "696e74323536206d616b6572416d6f756e742c75696e743235362074616b6572",
            "416d6f756e742c75696e743820736964652c75696e7438207369676e61747572",
            "65547970652c75696e743235362074696d657374616d702c6279746573333220",
            "6d657461646174612c62797465733332206275696c6465722900ba"
        );

        let signer: PrivateKeySigner = PRIVATE_KEY.parse().expect("valid test key");
        let exchange = Address::from_str(V2_EXCHANGE).expect("valid address");
        let deposit = Address::from_str(DEPOSIT_WALLET).expect("valid address");

        // Order matches the SDK fixture verbatim:
        //   salt=479_249_096_354, tokenId=1234, makerAmt=100_000_000,
        //   takerAmt=50_000_000, side=Buy(0), sigType=POLY_1271(3),
        //   timestamp_ms=1_710_000_000_000, metadata=0, builder=0
        let order = SignedOrderMessage {
            salt: U256::from(479_249_096_354_u64),
            maker: deposit,
            signer: deposit,
            token_id: U256::from(1234_u64),
            maker_amount: U256::from(100_000_000_u64),
            taker_amount: U256::from(50_000_000_u64),
            side: 0,
            signature_type: 3,
            timestamp: U256::from(1_710_000_000_000_u64),
            metadata: B256::ZERO,
            builder: B256::ZERO,
        };

        let sig = sign_order_message(&signer, order, AMOY, exchange).expect("POLY_1271 signing");

        assert_eq!(
            sig, EXPECTED,
            "wrapped POLY_1271 signature drifted from rs-clob-client-v2 reference"
        );
        // 0x + 130 (inner sig) + 64 (domain sep) + 64 (contents hash)
        // + 186*2 (ORDER_TYPE_STRING bytes) + 4 (u16 length) = 2 + 130 + 64 + 64 + 372 + 4 = 636.
        assert_eq!(
            sig.len(),
            2 + 130 + 64 + 64 + (ORDER_TYPE_STRING.len() * 2) + 4
        );
    }
}
