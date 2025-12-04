#!/bin/bash

# Network benchmark script for polyfill-rs
# Tests real network latency vs computational performance

set -e

echo "🌐 Network Latency Benchmark for polyfill-rs"
echo "============================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

print_header() {
    echo -e "${BLUE}$1${NC}"
}

print_success() {
    echo -e "${GREEN}✅ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

print_error() {
    echo -e "${RED}❌ $1${NC}"
}

# Test network connectivity
test_connectivity() {
    print_header "Testing Polymarket API connectivity..."
    
    if curl -s --max-time 10 "https://clob.polymarket.com/ok" > /dev/null; then
        print_success "Polymarket API is reachable"
        return 0
    else
        print_error "Cannot reach Polymarket API"
        return 1
    fi
}

# Run network benchmarks
run_network_benchmarks() {
    print_header "Running network benchmarks..."
    
    echo "Building benchmarks..."
    cargo build --release --benches
    
    echo "Running network latency tests..."
    cargo bench --bench network_benchmarks
    
    print_success "Network benchmarks completed"
}

# Compare with computational benchmarks
run_comparison() {
    print_header "Running computational vs network comparison..."
    
    echo "1. Computational benchmarks (no network):"
    cargo bench --bench comparison_benchmarks
    
    echo ""
    echo "2. Network benchmarks (with real API calls):"
    cargo bench --bench network_benchmarks
    
    print_success "Comparison completed"
}

# Manual timing test
manual_timing_test() {
    print_header "Manual timing test..."
    
    echo "Testing real API calls with manual timing:"
    
    cat > /tmp/timing_test.rs << 'EOF'
use polyfill_rs::ClobClient;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClobClient::new("https://clob.polymarket.com");
    
    println!("Testing simplified markets endpoint...");
    let start = Instant::now();
    match client.get_sampling_simplified_markets(None).await {
        Ok(markets) => {
            let duration = start.elapsed();
            println!("✅ Fetched {} markets in {:?}", markets.data.len(), duration);
        }
        Err(e) => {
            let duration = start.elapsed();
            println!("❌ Error after {:?}: {}", duration, e);
        }
    }
    
    println!("\nTesting full markets endpoint...");
    let start = Instant::now();
    match client.get_sampling_markets(None).await {
        Ok(markets) => {
            let duration = start.elapsed();
            println!("✅ Fetched {} markets in {:?}", markets.data.len(), duration);
        }
        Err(e) => {
            let duration = start.elapsed();
            println!("❌ Error after {:?}: {}", duration, e);
        }
    }
    
    Ok(())
}
EOF

    echo "Compiling timing test..."
    rustc --edition 2021 -L target/release/deps /tmp/timing_test.rs -o /tmp/timing_test \
        --extern polyfill_rs=target/release/libpolyfill_rs.rlib \
        --extern tokio=target/release/deps/libtokio*.rlib 2>/dev/null || {
        print_warning "Could not compile timing test. Running with cargo instead..."
        
        # Create a simple example instead
        cargo run --release --example benchmark_demo | grep -E "(Fetch|Average|Error)"
        return
    }
    
    echo "Running timing test..."
    /tmp/timing_test
    
    print_success "Manual timing test completed"
}

# Generate network vs computational report
generate_network_report() {
    print_header "Generating network performance report..."
    
    cat << 'EOF'

📊 NETWORK vs COMPUTATIONAL PERFORMANCE
=======================================

To get fair benchmarks against polymarket-rs-client, we need to measure:

1. **Computational Performance** (what we currently measure):
   - JSON parsing: ~2.4µs
   - Order creation: ~19.7ns (struct creation only)
   - Order book ops: ~118µs per 1000 updates

2. **Network Performance** (what original benchmarks measure):
   - Full HTTP request + JSON parsing
   - EIP-712 signing + network round-trip
   - Real-world latency including server response time

3. **Fair Comparison Requirements**:
   - Same network conditions (geographic location)
   - Same API endpoints and request patterns
   - Same authentication and signing overhead

🔬 **To run network benchmarks**:
   ./scripts/network_benchmark.sh

📈 **Expected results**:
   - Network latency will dominate (100-500ms typical)
   - Our computational improvements still provide benefits
   - Total time = Network Time + Computational Time
   - We optimize the computational portion significantly

🎯 **Real-world impact**:
   - In co-located environments: computational speed matters more
   - Over internet: network dominates, but every microsecond counts
   - For high-frequency operations: our optimizations compound

EOF

    print_success "Report generated"
}

# Main execution
main() {
    if ! test_connectivity; then
        print_error "Cannot proceed without network connectivity"
        generate_network_report
        exit 1
    fi
    
    # Build first
    print_header "Building polyfill-rs..."
    cargo build --release
    
    # Run tests
    manual_timing_test
    echo ""
    run_network_benchmarks
    echo ""
    generate_network_report
    
    print_success "Network benchmark analysis completed!"
    echo ""
    echo "📁 Detailed results in target/criterion/*/report/index.html"
    echo "🔗 Compare with: https://github.com/polymarket-rs-client"
}

# Run main function
main "$@"
