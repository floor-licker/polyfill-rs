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
- **Dependencies**: Network connectivity, optional authentication credentials
- **Speed**: Slower (network calls)

## Running Tests

### Quick Start (Basic Tests)
```bash
# Run all unit tests
cargo test

# Run only integration tests
cargo test --test integration_tests

# Run with verbose output
cargo test --test integration_tests -- --nocapture
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
# Using the test runner script
./scripts/run_integration_tests.sh

# Or directly with cargo
cargo test --test integration_tests -- --nocapture
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

### Skip Indicators
```
Skipping authentication test - no private key provided
Skipping order management test - missing auth credentials
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

# Test with minimal credentials
export POLYMARKET_PRIVATE_KEY="0x1234567890123456789012345678901234567890123456789012345678901234"
cargo test test_authentication
```

#### Rate Limiting
```bash
# If tests fail due to rate limiting, add delays
export POLYMARKET_TEST_DELAY=1000  # 1 second between requests
```

### Debug Mode

Run tests with detailed logging:

```bash
# Enable debug logging
RUST_LOG=debug cargo test --test integration_tests -- --nocapture

# Enable trace logging for maximum detail
RUST_LOG=trace cargo test --test integration_tests -- --nocapture
```

## Continuous Integration

### GitHub Actions

Our CI runs integration tests automatically:

```yaml
# .github/workflows/ci.yml
- name: Run Integration Tests
  env:
    POLYMARKET_HOST: ${{ secrets.POLYMARKET_HOST }}
    POLYMARKET_CHAIN_ID: ${{ secrets.POLYMARKET_CHAIN_ID }}
  run: cargo test --test integration_tests
```

### Local CI

Run the same tests locally:

```bash
# Install cargo-nextest for faster test execution
cargo install cargo-nextest

# Run with nextest
cargo nextest run --test integration_tests
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