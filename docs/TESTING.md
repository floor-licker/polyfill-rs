# Testing polyfill-rs

This document describes how to run tests for polyfill-rs, with a focus on integration tests that verify our client can actually communicate with the real Polymarket API.

## Test Types

### Unit Tests
- **Location**: Scattered throughout source files (`src/*.rs`)
- **Purpose**: Test individual functions and components in isolation
- **Dependencies**: None (pure functions)
- **Speed**: Fast

### Integration Tests
- **Location**: `tests/integration_tests.rs`
- **Purpose**: Verify the client can communicate with the real Polymarket API
- **Dependencies**: Network connectivity + credentials (tests are `#[ignore]` by default)
- **Speed**: Slower (network calls)

## Running Tests

### Quick Start (Basic Tests)
```bash
# Run unit tests + doc tests (real-API tests are `#[ignore]` by default)
cargo test --all-features

# Run the "no-alloc hot paths" regression tests
cargo test --all-features --test no_alloc_hot_paths

# Compile-check all examples
cargo build --examples
```

### Full Integration Testing

#### 1. Set up Environment Variables

Create a `.env` file or export variables:

```bash
# Required for authentication tests
export POLYMARKET_PRIVATE_KEY="your_private_key_here"

# Required for order management tests
export POLYMARKET_API_KEY="your_api_key"
export POLYMARKET_API_SECRET="your_api_secret"
export POLYMARKET_API_PASSPHRASE="your_passphrase"

# Optional (defaults provided)
export POLYMARKET_HOST="https://clob.polymarket.com"
export POLYMARKET_CHAIN_ID="137"
```

#### 2. Run Integration Tests

```bash
# Using the test runner script (runs ignored tests that hit the real API)
./scripts/run_integration_tests.sh

# Or directly with cargo
cargo test --all-features --test integration_tests -- --ignored --nocapture --test-threads=1
```

## Test Categories

### Always Run (No Auth Required)
- **API Connectivity**: Basic connection to Polymarket API
- **Market Data Endpoints**: Order book, prices, spreads, etc.
- **Error Handling**: Invalid requests and error responses
- **Rate Limiting**: Multiple rapid requests
- **API Compatibility**: Verify our API matches polymarket-rs-client
- **Performance**: Response time measurements

### Authentication Required
- **Authentication**: API key creation and validation
- **Advanced Client Features**: Full client configuration
- **WebSocket Connectivity**: Real-time data streaming

### API Credentials Required
- **Order Management**: Order creation and management (read-only tests)

## Test Results

### Success Indicators
```
API connectivity test passed
Market data endpoints test passed
Error handling test passed
Rate limiting test passed
API compatibility test passed
Performance test passed
  Server time: 234ms
  Markets request: 1.2s
  Markets returned: 50
```

### Ignored Indicators (default)
```
test test_real_api_* ... ignored
```

### Failure Indicators
```
API connectivity test failed: Network error: connection refused
Market data endpoints test failed: API error (404): Token not found
```

## Performance Benchmarks

Our integration tests include performance measurements:

| Operation | Expected Time | Actual Time |
|-----------|---------------|-------------|
| Server Time | < 5s | 234ms |
| Markets Request | < 10s | 1.2s |
| Order Book | < 5s | 890ms |
| Price Quote | < 3s | 156ms |

## Troubleshooting

### Common Issues

#### Network Connectivity
```bash
# Test basic connectivity
curl -I https://clob.polymarket.com/

# Check DNS resolution
nslookup clob.polymarket.com
```

#### Authentication Issues
```bash
# Verify private key format
echo $POLYMARKET_PRIVATE_KEY | wc -c  # Should be 66 characters (0x + 64 hex)

# Run a small ignored auth smoke-test (requires real credentials)
cargo test --all-features --test simple_auth_test -- --ignored --nocapture --test-threads=1
```

#### Rate Limiting
```bash
# If tests fail due to rate limiting, consider adding delays between manual runs.
```

### Debug Mode

Run tests with detailed logging:

```bash
# Enable debug logging
RUST_LOG=debug cargo test --all-features --test integration_tests -- --ignored --nocapture --test-threads=1

# Enable trace logging for maximum detail
RUST_LOG=trace cargo test --all-features --test integration_tests -- --ignored --nocapture --test-threads=1
```

## Continuous Integration

### GitHub Actions

Our CI runs formatting, clippy, unit tests, docs, security audit, and a separate no-alloc job. Real-API integration tests are `#[ignore]` and are not run in CI.

```yaml
# .github/workflows/ci.yml
- name: Run tests (excluding no-alloc hot paths)
  run: cargo test --all-features -- --skip no_alloc_

- name: Run no-alloc hot path tests
  run: cargo test --all-features --test no_alloc_hot_paths
```

### Local CI

Run the same tests locally:

```bash
# Install cargo-nextest for faster test execution
cargo install cargo-nextest

# Run with nextest (ignored tests are not run by default)
cargo nextest run --all-features
```

## Test Coverage

Our integration tests cover:

- **API Endpoints**: All major REST endpoints
- **Authentication**: EIP-712 signing and API key management
- **Error Handling**: Network errors, API errors, validation errors
- **Performance**: Response time and throughput measurements
- **WebSocket**: Real-time data streaming (when available)
- **Compatibility**: API compatibility with polymarket-rs-client

## Adding New Tests

### Template for New Integration Test

```rust
#[tokio::test]
async fn test_new_feature() -> Result<()> {
    let config = TestConfig::from_env();
    
    // Skip if requirements not met
    if !config.has_auth() {
        TestReporter::skip("test_new_feature", "no private key");
        return Ok(());
    }
    
    // Test implementation
    let client = config.create_auth_client()?;
    let result = client.some_new_method().await?;
    
    // Assertions
    assert!(result.is_valid());
    
    TestReporter::success("test_new_feature");
    Ok(())
}
```

### Best Practices

1. **Use TestConfig**: Always use the shared test configuration
2. **Handle Missing Credentials**: Skip tests gracefully when credentials aren't available
3. **Measure Performance**: Include timing measurements for performance-critical operations
4. **Provide Context**: Use descriptive test names and error messages
5. **Clean Up**: Don't leave test data in the system

## Security Notes

- **Never commit credentials**: All test credentials are loaded from environment variables
- **Use test accounts**: If testing with real credentials, use dedicated test accounts
- **Read-only tests**: Order management tests only create orders, they don't execute them
- **Rate limiting**: Tests include delays to respect API rate limits 
