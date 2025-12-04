#!/bin/bash

# Benchmark comparison script for polyfill-rs vs polymarket-rs-client
# This script runs benchmarks and measures memory usage

set -e

echo "🚀 Running polyfill-rs benchmark comparison..."
echo "================================================"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print colored output
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

# Check if required tools are installed
check_dependencies() {
    print_header "Checking dependencies..."
    
    if ! command -v cargo &> /dev/null; then
        print_error "cargo not found. Please install Rust."
        exit 1
    fi
    
    if ! command -v valgrind &> /dev/null; then
        print_warning "valgrind not found. Memory profiling will be limited."
        VALGRIND_AVAILABLE=false
    else
        VALGRIND_AVAILABLE=true
    fi
    
    print_success "Dependencies checked"
}

# Run criterion benchmarks
run_criterion_benchmarks() {
    print_header "Running Criterion benchmarks..."
    
    echo "Building benchmarks..."
    cargo build --release --benches
    
    echo "Running comparison benchmarks..."
    cargo bench --bench comparison_benchmarks
    
    print_success "Criterion benchmarks completed"
}

# Run memory profiling
run_memory_profiling() {
    print_header "Running memory profiling..."
    
    if [ "$VALGRIND_AVAILABLE" = true ]; then
        echo "Running with Valgrind (detailed memory analysis)..."
        
        # Create a simple test binary for memory profiling
        cat > /tmp/memory_test.rs << 'EOF'
use polyfill_rs::ClobClient;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClobClient::new("https://clob.polymarket.com");
    
    // Test memory usage for market fetching
    match client.get_sampling_markets(None).await {
        Ok(markets) => {
            println!("Fetched {} markets", markets.data.len());
            
            // Process markets to simulate real usage
            for market in &markets.data {
                let _ = &market.condition_id;
                let _ = &market.question;
            }
        }
        Err(e) => {
            println!("Error (expected without API key): {}", e);
        }
    }
    
    Ok(())
}
EOF
        
        # Compile the test
        rustc --edition 2021 -L target/release/deps /tmp/memory_test.rs -o /tmp/memory_test \
            --extern polyfill_rs=target/release/libpolyfill_rs.rlib \
            --extern tokio=target/release/deps/libtokio*.rlib 2>/dev/null || {
            print_warning "Could not compile memory test. Skipping detailed memory analysis."
            return
        }
        
        # Run with Valgrind
        valgrind --tool=massif --massif-out-file=/tmp/massif.out /tmp/memory_test 2>/dev/null || {
            print_warning "Valgrind analysis failed. This is normal without network access."
        }
        
        if [ -f /tmp/massif.out ]; then
            echo "Memory usage summary:"
            ms_print /tmp/massif.out | head -20
        fi
        
    else
        print_warning "Valgrind not available. Using basic memory monitoring."
        
        # Use built-in time command for basic memory stats
        echo "Running basic memory test..."
        /usr/bin/time -l cargo run --release --example quick_demo 2>&1 | grep -E "(maximum resident|peak memory)" || true
    fi
    
    print_success "Memory profiling completed"
}

# Generate comparison report
generate_report() {
    print_header "Generating comparison report..."
    
    cat << 'EOF'

📊 BENCHMARK COMPARISON REPORT
==============================

Based on the original polymarket-rs-client benchmarks:

| Operation | polymarket-rs-client | polyfill-rs | Improvement |
|-----------|---------------------|-------------|-------------|
| Create EIP-712 order | 266.5ms ± 28.6ms | [Run benchmarks] | [TBD] |
| Fetch simplified markets | 404.5ms ± 22.9ms | [Run benchmarks] | [TBD] |
| Memory usage (markets) | 15.9MB allocated | [Run benchmarks] | [TBD] |

🎯 Expected improvements with polyfill-rs:
- Fixed-point arithmetic reduces computational overhead
- Zero-allocation hot paths minimize GC pressure  
- Optimized data structures reduce memory footprint
- Cache-friendly memory layouts improve performance

📈 To get actual numbers:
1. Run: ./scripts/benchmark_comparison.sh
2. Check: target/criterion/*/report/index.html
3. Compare with baseline numbers above

EOF

    print_success "Report generated"
}

# Main execution
main() {
    check_dependencies
    
    # Build the project first
    print_header "Building polyfill-rs in release mode..."
    cargo build --release
    
    run_criterion_benchmarks
    run_memory_profiling
    generate_report
    
    print_success "Benchmark comparison completed!"
    echo ""
    echo "📁 Detailed results available in:"
    echo "   - target/criterion/*/report/index.html (interactive charts)"
    echo "   - Criterion output above (summary statistics)"
    echo ""
    echo "🔗 Compare with baseline: https://github.com/polymarket-rs-client"
}

# Run main function
main "$@"
