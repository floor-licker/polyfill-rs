#!/bin/bash

# Demo runner for polyfill-rs
# This script runs the quick demo to showcase all available endpoints

set -e

echo "Running polyfill-rs Quick Demo"
echo "================================="

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ]; then
    echo " Error: Please run this script from the project root directory"
    exit 1
fi

# Check if the demo example exists
if [ ! -f "examples/quick_demo.rs" ]; then
    echo " Error: Quick demo example not found"
    exit 1
fi

echo "Configuration:"
echo "  Host: https://clob.polymarket.com"
echo "  Chain ID: 137 (Polygon)"
echo "  Authentication: None required (public endpoints only)"
echo ""

# Run the quick demo
echo "Running quick demo..."
echo "===================="

cargo run --example quick_demo

if [ $? -eq 0 ]; then
    echo ""
    echo " Quick demo completed successfully!"
    echo ""
    echo "What was tested:"
    echo "   API connectivity (/ok, /time)"
    echo "   Market data retrieval (/sampling-markets)"
    echo "   Order book data (/book)"
    echo "   Price data (/midpoint, /spread, /price)"
    echo "   Market metadata (/tick-size, /neg-risk)"
    echo "   Error handling (invalid inputs)"
    echo "   Performance characteristics"
    echo "   Data consistency validation"
    echo ""
    echo "The polyfill-rs client is working correctly with the Polymarket API!"
else
    echo ""
    echo " Quick demo failed!"
    echo "Check the output above for error details."
    exit 1
fi
