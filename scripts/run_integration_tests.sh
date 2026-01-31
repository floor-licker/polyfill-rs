#!/usr/bin/env bash
#
# Runs the real-API integration tests for polyfill-rs.
#
# These tests hit the live Polymarket API and are `#[ignore]` by default.
# You must provide credentials via environment variables or a local `.env` file.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "Running ignored integration tests against the live Polymarket API..."

if [[ -z "${POLYMARKET_PRIVATE_KEY:-}" ]]; then
  if [[ -f .env ]] && grep -q '^POLYMARKET_PRIVATE_KEY=' .env; then
    echo "Using POLYMARKET_PRIVATE_KEY from .env"
  else
    echo "ERROR: POLYMARKET_PRIVATE_KEY is not set (env or .env)."
    echo "Set POLYMARKET_PRIVATE_KEY in your environment or add it to .env."
    exit 1
  fi
fi

set -x

# Run serially to reduce the chance of hitting rate limits.
cargo test --all-features --test integration_tests -- --ignored --nocapture --test-threads=1
cargo test --all-features --test simple_auth_test -- --ignored --nocapture --test-threads=1
cargo test --all-features --test order_posting_test -- --ignored --nocapture --test-threads=1
