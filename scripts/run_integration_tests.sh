#!/bin/bash

# Integration test runner for polyfill-rs
# This script runs comprehensive integration tests against the real Polymarket API

set -e

echo "Running polyfill-rs integration tests..."
echo "=========================================="

# Check if we have the required environment variables
if [ -z "$POLYMARKET_PRIVATE_KEY" ]; then
    echo "Warning: POLYMARKET_PRIVATE_KEY not set"
    echo "   Some tests will be skipped (authentication, order management, WebSocket)"
    echo "   Set POLYMARKET_PRIVATE_KEY to run all tests"
fi

if [ -z "$POLYMARKET_API_KEY" ] || [ -z "$POLYMARKET_API_SECRET" ] || [ -z "$POLYMARKET_API_PASSPHRASE" ]; then
    echo "Warning: API credentials not set"
    echo "   Set POLYMARKET_API_KEY, POLYMARKET_API_SECRET, and POLYMARKET_API_PASSPHRASE"
    echo "   to test order management functionality"
fi

# Set default values for optional environment variables
export POLYMARKET_HOST=${POLYMARKET_HOST:-"https://clob.polymarket.com"}
export POLYMARKET_CHAIN_ID=${POLYMARKET_CHAIN_ID:-"137"}

echo "Configuration:"
echo "  Host: $POLYMARKET_HOST"
echo "  Chain ID: $POLYMARKET_CHAIN_ID"
echo "  Has Auth: $([ -n "$POLYMARKET_PRIVATE_KEY" ] && echo "Yes" || echo "No")"
echo "  Has API Creds: $([ -n "$POLYMARKET_API_KEY" ] && echo "Yes" || echo "No")"
echo ""

# Run the tests
echo "Running integration tests..."
cargo test --test integration_tests -- --nocapture

if [ $? -eq 0 ]; then
    echo ""
    echo "All integration tests passed!"
    echo ""
    echo "Test Summary:"
    echo "  API connectivity"
    echo "  Market data endpoints"
    echo "  Error handling"
    echo "  Rate limiting"
    echo "  API compatibility"
    echo "  Performance characteristics"
    
    if [ -n "$POLYMARKET_PRIVATE_KEY" ]; then
        echo "  Authentication"
        echo "  Advanced client features"
        echo "  WebSocket connectivity"
        
        if [ -n "$POLYMARKET_API_KEY" ]; then
            echo "  Order management"
        else
            echo "  Order management (skipped - no API credentials)"
        fi
    else
        echo "  Authentication (skipped - no private key)"
        echo "  Advanced client features (skipped - no private key)"
        echo "  WebSocket connectivity (skipped - no private key)"
        echo "  Order management (skipped - no private key)"
    fi
else
    echo ""
    echo "Some integration tests failed!"
    exit 1
fi 