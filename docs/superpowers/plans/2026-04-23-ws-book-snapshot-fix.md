# WS `book` Snapshot + Millis Timestamp Fix — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two bugs in polyfill2's WebSocket `book` message handling — timestamp parsed as seconds instead of millis, and `book` message treated as delta instead of full snapshot — while preserving the project's zero-allocation contract on the hot path.

**Architecture:** Mark-and-sweep in-place on `BTreeMap<Price, Qty>` values. Phase 1 (mark) walks `values_mut()` zeroing existing sizes. Phase 2 (apply) overwrites surviving prices. Phase 3 (sweep) drops remaining zeros via `retain`. No `BTreeMap::clear()` — preserves node allocations when price ladder is stable.

**Tech Stack:** Rust 2021, chrono 0.4.44 (has `from_timestamp_millis`), simd-json, tokio, BTreeMap. Test runner: `cargo test` under `devenv shell --`. Pre-commit hooks: rustfmt, clippy.

**Spec:** `docs/superpowers/specs/2026-04-23-ws-book-snapshot-fix-design.md`
**Issue:** https://github.com/onsails/polyfill2-rs/issues/6
**Target release:** v0.3.0

---

## Files Touched

**Created:**
- `tests/book_snapshot_tests.rs` — focused regression + debug_assert suite
- `CHANGELOG.md` — new changelog (none exists today)

**Modified:**
- `src/book.rs` — `begin_ws_book_update`, `finish_ws_book_update`, `apply_book_update`
- `src/ws_hot_path.rs::apply_levels` — rolling debug_assert on input ordering
- `tests/no_alloc_hot_paths.rs` — new steady-state snapshot replay tests
- `Cargo.toml` — version bump 0.2.0 → 0.3.0

**Unchanged** (mentioned for clarity):
- `src/types.rs` (BookUpdate, OrderSummary, BookLevel untouched)
- `src/decode.rs` (REST timestamps correctly parsed as seconds)
- `src/client.rs`, `src/stream.rs` (no surface change)
- All other tests

---

## Task 1: Setup worktree + feature branch

**Files:**
- Create worktree: `.worktrees/issue-6-ws-book-snapshot/`

- [ ] **Step 1: Create the worktree with a new branch**

```bash
cd /home/wb/dev/polyfill-rs
git worktree add .worktrees/issue-6-ws-book-snapshot -b fix/issue-6-ws-book-snapshot main
```

- [ ] **Step 2: Verify worktree is on the correct branch**

```bash
cd /home/wb/dev/polyfill-rs/.worktrees/issue-6-ws-book-snapshot
git rev-parse --abbrev-ref HEAD
```

Expected output: `fix/issue-6-ws-book-snapshot`

- [ ] **Step 3: Baseline check — full test suite is green**

```bash
cd /home/wb/dev/polyfill-rs/.worktrees/issue-6-ws-book-snapshot
devenv shell -- cargo test --all-features 2>&1 | tail -40
```

Expected: all tests pass (existing behavior). This is our green baseline.

**All subsequent tasks run inside `/home/wb/dev/polyfill-rs/.worktrees/issue-6-ws-book-snapshot/`.**

---

## Task 2: Add regression tests (Commit 1, TDD failing-tests baseline)

**Files:**
- Create: `tests/book_snapshot_tests.rs`

Tests that currently fail are gated with `#[ignore]`. Tests that already pass (documenting existing correct behavior) run immediately.

- [ ] **Step 1: Create the test file with all 8 regression tests**

Create `tests/book_snapshot_tests.rs` with exactly this content:

```rust
//! Regression tests for issue #6 — WS `book` snapshot semantics + millis timestamp.
//!
//! See `docs/superpowers/specs/2026-04-23-ws-book-snapshot-fix-design.md`.

use std::str::FromStr;

use chrono::Datelike;
use polyfill2::types::{BookUpdate, OrderSummary};
use polyfill2::OrderBookImpl;
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn level(price: &str, size: &str) -> OrderSummary {
    OrderSummary {
        price: dec(price),
        size: dec(size),
    }
}

fn book_update(
    asset_id: &str,
    timestamp: u64,
    bids: Vec<OrderSummary>,
    asks: Vec<OrderSummary>,
) -> BookUpdate {
    BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp,
        bids,
        asks,
        hash: None,
    }
}

const ASSET: &str = "test_asset_id";

/// Bug B regression: a new `book` snapshot must remove levels from prior snapshots
/// that are not present in the new message.
#[test]
#[ignore = "unignored in Task 4 (bug B fix)"]
fn snapshot_clears_stale_levels() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);

    // S1: bids at 0.74 and 0.75; asks at 0.76 and 0.77.
    book.apply_book_update(&book_update(
        ASSET,
        1_000_000_000_001,
        vec![level("0.74", "100"), level("0.75", "200")],
        vec![level("0.76", "50"), level("0.77", "30")],
    ))
    .unwrap();

    // S2: only 0.80 bid and 0.81 ask — S1's levels must disappear.
    book.apply_book_update(&book_update(
        ASSET,
        1_000_000_000_002,
        vec![level("0.80", "150")],
        vec![level("0.81", "25")],
    ))
    .unwrap();

    let snap = book.snapshot();
    assert_eq!(snap.bids.len(), 1, "stale bids leaked: {:?}", snap.bids);
    assert_eq!(snap.asks.len(), 1, "stale asks leaked: {:?}", snap.asks);
    assert_eq!(snap.bids[0].price, dec("0.80"));
    assert_eq!(snap.asks[0].price, dec("0.81"));
}

/// Bug A regression: a 13-digit millisecond timestamp must parse to the current century,
/// not to year ~57,716 (which is what happens when millis are interpreted as seconds).
#[test]
#[ignore = "unignored in Task 3 (bug A fix)"]
fn snapshot_timestamp_parses_as_millis() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);

    // 2025-09-15T03:21:32.351Z in milliseconds.
    let ts_millis: u64 = 1_757_908_892_351;

    book.apply_book_update(&book_update(
        ASSET,
        ts_millis,
        vec![level("0.75", "100")],
        vec![level("0.76", "100")],
    ))
    .unwrap();

    let snap = book.snapshot();
    let year = snap.timestamp.year();
    assert!(
        (2020..2100).contains(&year),
        "timestamp parsed as seconds instead of millis: got year {year}",
    );
}

/// A snapshot carrying zero-sized wire levels must not place them in the book.
#[test]
fn snapshot_drops_zero_sized_levels() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);

    book.apply_book_update(&book_update(
        ASSET,
        1_000_000_000_001,
        vec![level("0.74", "0"), level("0.75", "100")],
        vec![level("0.76", "50"), level("0.77", "0")],
    ))
    .unwrap();

    let snap = book.snapshot();
    assert!(
        snap.bids.iter().all(|l| l.price != dec("0.74")),
        "zero-sized bid survived: {:?}",
        snap.bids,
    );
    assert!(
        snap.asks.iter().all(|l| l.price != dec("0.77")),
        "zero-sized ask survived: {:?}",
        snap.asks,
    );
    assert_eq!(snap.bids.len(), 1);
    assert_eq!(snap.asks.len(), 1);
}

/// max_depth is enforced: if the snapshot contains more levels than max_depth,
/// only the best levels are retained (highest bids, lowest asks).
#[test]
fn snapshot_enforces_max_depth_keeping_best() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 3);

    // 5 bids ascending (0.70..0.74); 5 asks ascending (0.80..0.84).
    // Best bid = 0.74 (highest). Best ask = 0.80 (lowest).
    let bids: Vec<_> = (0..5).map(|i| level(&format!("0.7{i}"), "100")).collect();
    let asks: Vec<_> = (0..5).map(|i| level(&format!("0.8{i}"), "100")).collect();

    book.apply_book_update(&book_update(ASSET, 1_000_000_000_001, bids, asks))
        .unwrap();

    let snap = book.snapshot();
    assert_eq!(snap.bids.len(), 3, "bids exceed max_depth");
    assert_eq!(snap.asks.len(), 3, "asks exceed max_depth");

    let bid_prices: Vec<_> = snap.bids.iter().map(|l| l.price).collect();
    let ask_prices: Vec<_> = snap.asks.iter().map(|l| l.price).collect();
    assert!(
        bid_prices.contains(&dec("0.74")),
        "best bid dropped: {bid_prices:?}",
    );
    assert!(
        ask_prices.contains(&dec("0.80")),
        "best ask dropped: {ask_prices:?}",
    );
}

/// A snapshot whose timestamp is <= the book's current sequence is discarded
/// without mutating the book.
#[test]
fn snapshot_ignored_when_timestamp_le_sequence() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);

    book.apply_book_update(&book_update(
        ASSET,
        10,
        vec![level("0.75", "100")],
        vec![level("0.76", "100")],
    ))
    .unwrap();

    // Stale snapshot: timestamp (5) < current sequence (10).
    book.apply_book_update(&book_update(
        ASSET,
        5,
        vec![level("0.99", "999")],
        vec![level("0.01", "999")],
    ))
    .unwrap();

    let snap = book.snapshot();
    assert_eq!(snap.bids.len(), 1);
    assert_eq!(snap.bids[0].price, dec("0.75"));
    assert_eq!(snap.asks.len(), 1);
    assert_eq!(snap.asks[0].price, dec("0.76"));
}

/// Docs-example input shows ascending-price on both sides. In debug builds we
/// assert this and panic on violation to catch a server-side contract change early.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "ascending")]
#[ignore = "unignored in Task 4 (debug_assert added)"]
fn snapshot_panics_on_descending_bids_in_debug() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);
    book.apply_book_update(&book_update(
        ASSET,
        1,
        vec![level("0.75", "100"), level("0.74", "100")], // DESCENDING — violation
        vec![level("0.76", "100")],
    ))
    .unwrap();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "ascending")]
#[ignore = "unignored in Task 4 (debug_assert added)"]
fn snapshot_panics_on_descending_asks_in_debug() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);
    book.apply_book_update(&book_update(
        ASSET,
        1,
        vec![level("0.75", "100")],
        vec![level("0.77", "100"), level("0.76", "100")], // DESCENDING — violation
    ))
    .unwrap();
}

/// Alternating snapshots S1 -> S2 -> S1 must produce the expected book each time,
/// with no state leakage between snapshots.
#[test]
#[ignore = "unignored in Task 4 (bug B fix)"]
fn snapshot_alternating_s1_s2_s1_has_no_leakage() {
    let mut book = OrderBookImpl::new(ASSET.to_string(), 100);

    let s1_bids = vec![level("0.74", "100"), level("0.75", "200")];
    let s1_asks = vec![level("0.76", "50"), level("0.77", "30")];
    let s2_bids = vec![level("0.60", "500")];
    let s2_asks = vec![level("0.90", "10")];

    book.apply_book_update(&book_update(ASSET, 1, s1_bids.clone(), s1_asks.clone()))
        .unwrap();
    let snap1 = book.snapshot();
    assert_eq!(snap1.bids.len(), 2);
    assert_eq!(snap1.asks.len(), 2);

    book.apply_book_update(&book_update(ASSET, 2, s2_bids.clone(), s2_asks.clone()))
        .unwrap();
    let snap2 = book.snapshot();
    assert_eq!(snap2.bids.len(), 1);
    assert_eq!(snap2.bids[0].price, dec("0.60"));
    assert_eq!(snap2.asks.len(), 1);
    assert_eq!(snap2.asks[0].price, dec("0.90"));

    book.apply_book_update(&book_update(ASSET, 3, s1_bids.clone(), s1_asks.clone()))
        .unwrap();
    let snap3 = book.snapshot();
    assert_eq!(snap3.bids.len(), 2);
    assert_eq!(snap3.asks.len(), 2);
    assert!(
        snap3.bids.iter().all(|l| l.price != dec("0.60")),
        "S2 bid leaked into S3: {:?}",
        snap3.bids,
    );
    assert!(
        snap3.asks.iter().all(|l| l.price != dec("0.90")),
        "S2 ask leaked into S3: {:?}",
        snap3.asks,
    );
}
```

- [ ] **Step 2: Confirm the test file compiles + ignored tests are skipped + non-ignored pass**

```bash
devenv shell -- cargo test --all-features --test book_snapshot_tests 2>&1 | tail -20
```

Expected output excerpt:
- `test snapshot_drops_zero_sized_levels ... ok`
- `test snapshot_enforces_max_depth_keeping_best ... ok`
- `test snapshot_ignored_when_timestamp_le_sequence ... ok`
- 5 tests marked `ignored`
- `test result: ok. 3 passed; 0 failed; 5 ignored`

If any non-ignored test fails, STOP and diagnose before proceeding — it means our model of existing behavior is wrong.

- [ ] **Step 3: Confirm existing tests still green**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -5
```

Expected: `test result: ok` from every test binary. No `FAILED`.

- [ ] **Step 4: Commit**

```bash
git add tests/book_snapshot_tests.rs
git commit -m "$(cat <<'EOF'
test(book): add regression tests for ws book snapshot bugs

Adds failing tests for both bugs in issue #6 (gated behind #[ignore]
until their fix lands in later commits) and passing tests that codify
existing correct behavior (zero-sized drop, max_depth enforcement,
stale-sequence rejection).
EOF
)"
```

---

## Task 3: Fix Bug A — parse ws book timestamp as millis (Commit 2)

**Files:**
- Modify: `src/book.rs:417` and `src/book.rs:474`
- Modify: `tests/book_snapshot_tests.rs` (unignore timestamp test)

- [ ] **Step 1: Fix `begin_ws_book_update`**

In `src/book.rs`, replace line 417:

```rust
            chrono::DateTime::<Utc>::from_timestamp(timestamp as i64, 0).unwrap_or_else(Utc::now);
```

with:

```rust
            chrono::DateTime::<Utc>::from_timestamp_millis(timestamp as i64)
                .unwrap_or_else(Utc::now);
```

- [ ] **Step 2: Fix `apply_book_update`**

In `src/book.rs`, replace lines 474-475:

```rust
        self.timestamp = chrono::DateTime::<Utc>::from_timestamp(update.timestamp as i64, 0)
            .unwrap_or_else(Utc::now);
```

with:

```rust
        self.timestamp = chrono::DateTime::<Utc>::from_timestamp_millis(update.timestamp as i64)
            .unwrap_or_else(Utc::now);
```

- [ ] **Step 3: Unignore the timestamp regression test**

In `tests/book_snapshot_tests.rs`, replace:

```rust
#[test]
#[ignore = "unignored in Task 3 (bug A fix)"]
fn snapshot_timestamp_parses_as_millis() {
```

with:

```rust
#[test]
fn snapshot_timestamp_parses_as_millis() {
```

- [ ] **Step 4: Run the targeted test — it should pass now**

```bash
devenv shell -- cargo test --all-features --test book_snapshot_tests snapshot_timestamp_parses_as_millis 2>&1 | tail -10
```

Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Step 5: Run the full suite — everything green**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -10
```

Expected: all tests pass. 4 tests in `book_snapshot_tests` now pass (previous 3 + timestamp), 4 remain ignored.

- [ ] **Step 6: Commit**

```bash
git add src/book.rs tests/book_snapshot_tests.rs
git commit -m "$(cat <<'EOF'
fix(book): parse ws book timestamp as millis not seconds

Polymarket CLOB WSS `book` events carry a 13-digit millisecond
timestamp, but both begin_ws_book_update and apply_book_update used
from_timestamp(ts, 0) which interprets the value as seconds. A 2025
timestamp resolved to year ~57716.

Fixes part of #6. Upstream: floor-licker/polyfill-rs#23.
EOF
)"
```

---

## Task 4: Fix Bug B — mark-and-sweep snapshot semantics + debug_assert (Commit 3)

**Files:**
- Modify: `src/book.rs` (`begin_ws_book_update`, `finish_ws_book_update`, `apply_book_update`)
- Modify: `src/ws_hot_path.rs` (`apply_levels` — rolling debug_assert)
- Modify: `tests/book_snapshot_tests.rs` (unignore remaining 4 tests)

- [ ] **Step 1: Rewrite `begin_ws_book_update` with mark phase**

In `src/book.rs`, find the function at lines ~406-420. Replace the entire function body with:

```rust
    /// Begin applying a WebSocket `book` update (hot-path oriented).
    ///
    /// This is intended for in-place WS processing where we *stream* levels out of a decoded
    /// message, without constructing intermediate `BookUpdate` structs.
    ///
    /// Returns `Ok(true)` if the update should be applied, or `Ok(false)` if the update is stale
    /// and should be skipped.
    ///
    /// Since v0.3.0: the WS `book` event is a full snapshot (not a diff). This method
    /// performs the "mark" phase of mark-and-sweep: it zeros all existing level sizes so
    /// that `finish_ws_book_update` can sweep any level that the incoming message did not
    /// overwrite. See issue #6.
    pub(crate) fn begin_ws_book_update(&mut self, asset_id: &str, timestamp: u64) -> Result<bool> {
        if asset_id != self.token_id {
            return Err(PolyfillError::validation("Token ID mismatch"));
        }

        if timestamp <= self.sequence {
            return Ok(false);
        }

        self.sequence = timestamp;
        self.timestamp = chrono::DateTime::<Utc>::from_timestamp_millis(timestamp as i64)
            .unwrap_or_else(Utc::now);

        // Mark phase: zero every existing value in place. Phase 2 (apply) overwrites
        // survivors; phase 3 (sweep, in finish_ws_book_update) drops non-survivors.
        // `values_mut` iterates stack-only — no allocation.
        for size in self.bids.values_mut() {
            *size = 0;
        }
        for size in self.asks.values_mut() {
            *size = 0;
        }

        Ok(true)
    }
```

- [ ] **Step 2: Rewrite `finish_ws_book_update` with sweep phase**

In `src/book.rs`, find the function at lines ~446-449. Replace the entire function with:

```rust
    /// Finish applying a WS `book` update.
    ///
    /// Sweep phase of mark-and-sweep: drop any level still carrying sentinel size 0
    /// (either zeroed by `begin_ws_book_update` and not overwritten by the incoming
    /// snapshot, or carrying wire-zero size that should not appear in the book).
    /// Then enforce `max_depth`. `retain` is a no-op when nothing was marked stale.
    pub(crate) fn finish_ws_book_update(&mut self) {
        self.bids.retain(|_, size| *size != 0);
        self.asks.retain(|_, size| *size != 0);
        self.trim_depth();
    }
```

- [ ] **Step 3: Rewrite `apply_book_update` with mark-apply-sweep + debug_assert**

In `src/book.rs`, find `apply_book_update` at lines ~461-518. Replace the entire function with:

```rust
    /// Apply a full order-book snapshot from a WebSocket `book` event.
    ///
    /// **Semantics changed in 0.3.0:** previously upserted the supplied levels
    /// (preserved levels omitted from the message — wrong). Now replaces the book:
    /// levels omitted from the message are removed. Matches the actual wire
    /// contract of Polymarket CLOB V2 `book` messages. See issue #6.
    pub fn apply_book_update(&mut self, update: &BookUpdate) -> Result<()> {
        if update.asset_id != self.token_id {
            return Err(PolyfillError::validation("Token ID mismatch"));
        }

        if update.timestamp <= self.sequence {
            return Ok(());
        }

        // Polymarket WSS `book` events in the docs example carry ascending-price
        // levels on both sides. `trim_depth` below is order-agnostic so correctness
        // does not depend on this, but a violation signals a server-side contract
        // change worth catching early. Compiled out in release.
        debug_assert!(
            update.bids.windows(2).all(|w| w[0].price <= w[1].price),
            "CLOB `book` message: bids not in ascending price order. See polyfill2 issue #6.",
        );
        debug_assert!(
            update.asks.windows(2).all(|w| w[0].price <= w[1].price),
            "CLOB `book` message: asks not in ascending price order. See polyfill2 issue #6.",
        );

        self.sequence = update.timestamp;
        self.timestamp = chrono::DateTime::<Utc>::from_timestamp_millis(update.timestamp as i64)
            .unwrap_or_else(Utc::now);

        // Mark phase — see begin_ws_book_update for the model.
        for size in self.bids.values_mut() {
            *size = 0;
        }
        for size in self.asks.values_mut() {
            *size = 0;
        }

        // Apply phase — unconditional insert. Wire-zero levels are carried through
        // and dropped in the sweep. Overwriting an existing key does not allocate.
        for level in &update.bids {
            let price_ticks = decimal_to_price(level.price)
                .map_err(|_| PolyfillError::validation("Invalid price"))?;
            let size_units = decimal_to_qty(level.size)
                .map_err(|_| PolyfillError::validation("Invalid size"))?;

            if let Some(tick_size_ticks) = self.tick_size_ticks {
                if tick_size_ticks > 0 && !price_ticks.is_multiple_of(tick_size_ticks) {
                    return Err(PolyfillError::validation("Price not aligned to tick size"));
                }
            }

            self.bids.insert(price_ticks, size_units);
        }

        for level in &update.asks {
            let price_ticks = decimal_to_price(level.price)
                .map_err(|_| PolyfillError::validation("Invalid price"))?;
            let size_units = decimal_to_qty(level.size)
                .map_err(|_| PolyfillError::validation("Invalid size"))?;

            if let Some(tick_size_ticks) = self.tick_size_ticks {
                if tick_size_ticks > 0 && !price_ticks.is_multiple_of(tick_size_ticks) {
                    return Err(PolyfillError::validation("Price not aligned to tick size"));
                }
            }

            self.asks.insert(price_ticks, size_units);
        }

        // Sweep phase — drop marked-but-not-overwritten levels and any wire-zero.
        self.bids.retain(|_, size| *size != 0);
        self.asks.retain(|_, size| *size != 0);

        self.trim_depth();
        Ok(())
    }
```

- [ ] **Step 4: Add rolling debug_assert in `ws_hot_path::apply_levels`**

Open `src/ws_hot_path.rs`. Find the `apply_levels` function (around line 160). Replace it with:

```rust
fn apply_levels<'tape, 'input>(
    book: &mut crate::book::OrderBook,
    side: Side,
    levels: simd_json::tape::Array<'tape, 'input>,
) -> Result<usize> {
    let mut applied = 0usize;
    let mut prev_price: Option<crate::types::Price> = None;

    for level in levels.iter() {
        let Some(obj) = level.as_object() else {
            continue;
        };

        let price_str = obj
            .get("price")
            .and_then(|v| v.into_string())
            .ok_or_else(|| PolyfillError::parse("Missing price", None))?;
        let size_str = obj
            .get("size")
            .and_then(|v| v.into_string())
            .ok_or_else(|| PolyfillError::parse("Missing size", None))?;

        let price_decimal =
            Decimal::from_str(price_str).map_err(|_| PolyfillError::validation("Invalid price"))?;
        let size_decimal =
            Decimal::from_str(size_str).map_err(|_| PolyfillError::validation("Invalid size"))?;

        let price_ticks = decimal_to_price(price_decimal)
            .map_err(|_| PolyfillError::validation("Invalid price"))?;
        let size_units =
            decimal_to_qty(size_decimal).map_err(|_| PolyfillError::validation("Invalid size"))?;

        // Docs example shows ascending-price levels. Catch a server-side reorder in debug.
        // See polyfill2 issue #6.
        if let Some(prev) = prev_price {
            debug_assert!(
                price_ticks >= prev,
                "CLOB `book` message: levels not in ascending price order ({} < {}). See polyfill2 issue #6.",
                price_ticks,
                prev,
            );
        }
        prev_price = Some(price_ticks);

        book.apply_ws_book_level_fast(side, price_ticks, size_units)?;
        applied += 1;
    }

    Ok(applied)
}
```

- [ ] **Step 5: Unignore the 4 remaining bug-B and debug_assert tests**

In `tests/book_snapshot_tests.rs`, remove the `#[ignore = "..."]` line above each of these tests:
- `snapshot_clears_stale_levels`
- `snapshot_panics_on_descending_bids_in_debug`
- `snapshot_panics_on_descending_asks_in_debug`
- `snapshot_alternating_s1_s2_s1_has_no_leakage`

For each, change:

```rust
#[test]
#[ignore = "..."]
fn <test_name>() {
```

to:

```rust
#[test]
fn <test_name>() {
```

(Preserve `#[cfg(debug_assertions)]` and `#[should_panic(expected = "ascending")]` attributes where they exist.)

- [ ] **Step 6: Run the full `book_snapshot_tests` — all 8 should pass now**

```bash
devenv shell -- cargo test --all-features --test book_snapshot_tests 2>&1 | tail -20
```

Expected: `test result: ok. 8 passed; 0 failed; 0 ignored`.

- [ ] **Step 7: Run the existing no-alloc tests — all must stay green**

```bash
devenv shell -- cargo test --all-features --test no_alloc_hot_paths 2>&1 | tail -15
```

Expected: all 6 existing tests pass, `test result: ok. 6 passed; 0 failed`. This is non-negotiable — the zero-alloc contract on steady-state snapshot replay is the whole point of mark-and-sweep.

If a no-alloc test fails here, STOP. The mark-and-sweep implementation allocated where it shouldn't. Diagnose before proceeding.

- [ ] **Step 8: Run the full suite — everything green**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -10
```

Expected: all tests pass across every test binary.

- [ ] **Step 9: Clippy + build**

```bash
devenv shell -- cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -15
devenv shell -- cargo build --workspace 2>&1 | tail -5
```

Expected: no clippy warnings, clean build.

- [ ] **Step 10: Commit**

```bash
git add src/book.rs src/ws_hot_path.rs tests/book_snapshot_tests.rs
git commit -m "$(cat <<'EOF'
fix(book): treat ws book event as snapshot, not diff (mark-and-sweep)

The Polymarket CLOB V2 WSS `book` event carries a full order-book
snapshot, not an incremental diff. Pre-0.3.0 code upserted the supplied
levels without clearing the book, so levels from prior snapshots
accumulated and the local book drifted from exchange state.

Fix replaces upsert with mark-and-sweep on the BTreeMap values:

  Mark   — values_mut() -> 0 (stack-only iter, no alloc)
  Apply  — insert(price, size) (overwrites existing keys without alloc;
           allocates only for genuinely new prices)
  Sweep  — retain(|_, v| *v != 0) (deallocates only vanished levels)

Steady-state ladders (same prices, changed sizes) stay zero-alloc —
the existing no_alloc_hot_paths assertions remain green. Upstream PR
#24's naive `.clear()` + reinsert approach was rejected because it
breaks the zero-alloc contract on every message.

Also adds debug_assert! on ascending-price level ordering in both
apply_book_update and ws_hot_path::apply_levels. Compiled out in
release; catches a server-side contract change loudly in dev/CI.

Fixes #6. Ref: floor-licker/polyfill-rs#23.

BREAKING CHANGE: OrderBook::apply_book_update semantics changed from
upsert to full-snapshot replacement. Documented in CHANGELOG v0.3.0.
EOF
)"
```

---

## Task 5: Add steady-state no-alloc tests (Commit 4)

**Files:**
- Modify: `tests/no_alloc_hot_paths.rs` (append 4 new tests)

- [ ] **Step 1: Append new tests to `tests/no_alloc_hot_paths.rs`**

Open `tests/no_alloc_hot_paths.rs`. Append the following at the end of the file (after the last test, still at module scope — no `mod { ... }`):

```rust
/// Zero-alloc contract for snapshot replay: the same snapshot applied twice
/// must not allocate on the second apply (steady-state ladder).
#[test]
fn no_alloc_steady_state_snapshot_replay() {
    let asset_id = "test_asset_id";
    let mut book = OrderBookImpl::new(asset_id.to_string(), 100);

    let snapshot = polyfill2::types::BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp: 10,
        bids: vec![
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.74").unwrap(),
                size: Decimal::from_str("100").unwrap(),
            },
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.75").unwrap(),
                size: Decimal::from_str("200").unwrap(),
            },
        ],
        asks: vec![
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.76").unwrap(),
                size: Decimal::from_str("50").unwrap(),
            },
            polyfill2::types::OrderSummary {
                price: Decimal::from_str("0.77").unwrap(),
                size: Decimal::from_str("30").unwrap(),
            },
        ],
        hash: None,
    };

    // Warmup allocations allowed.
    book.apply_book_update(&snapshot).unwrap();

    // Replay with a strictly larger timestamp so the stale-sequence check passes.
    let replay = polyfill2::types::BookUpdate {
        timestamp: 11,
        ..snapshot.clone()
    };

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    book.apply_book_update(&replay).unwrap();
    guard.assert_no_allocations();
}

/// Same price ladder, different sizes — the common real-market case. Must not allocate.
#[test]
fn no_alloc_same_ladder_different_sizes() {
    let asset_id = "test_asset_id";
    let mut book = OrderBookImpl::new(asset_id.to_string(), 100);

    let make_snapshot = |ts: u64, bid_size: &str, ask_size: &str| polyfill2::types::BookUpdate {
        asset_id: asset_id.to_string(),
        market: "0xabc".to_string(),
        timestamp: ts,
        bids: vec![polyfill2::types::OrderSummary {
            price: Decimal::from_str("0.75").unwrap(),
            size: Decimal::from_str(bid_size).unwrap(),
        }],
        asks: vec![polyfill2::types::OrderSummary {
            price: Decimal::from_str("0.76").unwrap(),
            size: Decimal::from_str(ask_size).unwrap(),
        }],
        hash: None,
    };

    // Warmup.
    book.apply_book_update(&make_snapshot(10, "100", "50")).unwrap();

    // Same ladder, different sizes.
    let update = make_snapshot(11, "250", "75");

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    book.apply_book_update(&update).unwrap();
    guard.assert_no_allocations();
}

/// Steady-state snapshot replay through the simd-json hot path.
#[test]
fn no_alloc_same_ladder_via_ws_processor() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    let mut processor = WsBookUpdateProcessor::new(1024);

    // Warmup #1: decode + apply (allocates for simd-json buffers, BTreeMap nodes).
    let mut warmup = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"100\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50\"}}]}}"
    )
    .into_bytes();
    processor.process_bytes(warmup.as_mut_slice(), &manager).unwrap();

    // Second message: same ladder, newer timestamp, different sizes. Must be zero-alloc.
    let mut msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75\"}}]}}"
    )
    .into_bytes();

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    processor.process_bytes(msg.as_mut_slice(), &manager).unwrap();
    guard.assert_no_allocations();
}

/// Steady-state snapshot replay through the WebSocketStream book applier.
#[test]
fn no_alloc_same_ladder_via_ws_applier() {
    let asset_id = "test_asset_id";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    let processor = WsBookUpdateProcessor::new(1024);
    let stream = WebSocketStream::new("wss://example.com/ws");
    let mut applier = stream.into_book_applier(&manager, processor);

    let warmup = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":10,\"bids\":[{{\"price\":\"0.75\",\"size\":\"100\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"50\"}}]}}"
    );
    applier.apply_text_message(warmup).unwrap();

    let msg = format!(
        "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":11,\"bids\":[{{\"price\":\"0.75\",\"size\":\"200\"}}],\"asks\":[{{\"price\":\"0.76\",\"size\":\"75\"}}]}}"
    );

    let _ = allocation_count();
    let guard = NoAllocGuard::new();
    applier.apply_text_message(msg).unwrap();
    guard.assert_no_allocations();
}
```

- [ ] **Step 2: Run the no-alloc test binary — all 10 tests (6 pre-existing + 4 new) must pass**

```bash
devenv shell -- cargo test --all-features --test no_alloc_hot_paths 2>&1 | tail -20
```

Expected: `test result: ok. 10 passed; 0 failed`.

If the new tests fail with "expected no heap allocations, but saw N allocation(s)", investigate. The likely culprits:
- BTreeMap node allocation on a key that wasn't in the map (did warmup seed ALL keys the replay uses?)
- simd-json tape/buffers re-growing (warmup message must be at least as large as the measured message)
- Debug_assert formatting on failure (should not trigger — condition is true)

- [ ] **Step 3: Run full suite**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -10
```

Expected: all tests across all binaries pass.

- [ ] **Step 4: Commit**

```bash
git add tests/no_alloc_hot_paths.rs
git commit -m "$(cat <<'EOF'
test(no-alloc): add zero-alloc assertions for steady-state snapshot replay

Four new tests covering the hot-path zero-alloc contract under the new
mark-and-sweep snapshot semantics: direct OrderBook::apply_book_update,
same-ladder-different-sizes, and the WS decode+apply path through both
WsBookUpdateProcessor and WebSocketStream's book applier.

The six pre-existing no_alloc tests remain green unchanged — mark-and-
sweep preserves the zero-alloc guarantee when the price ladder is
stable, which is the common steady-state case.
EOF
)"
```

---

## Task 6: Add E2E docs-example tests (Commit 5)

**Files:**
- Modify: `tests/book_snapshot_tests.rs` (append E2E section)

We add self-contained E2E tests that exercise the full decode + apply path using a fixture payload derived from the Polymarket AsyncAPI docs example (documented in the spec). These do not hit the network, so they live alongside the regression tests rather than in `ws_integration_tests.rs` (which is gated for live-network tests).

- [ ] **Step 1: Append E2E tests to `tests/book_snapshot_tests.rs`**

At the end of `tests/book_snapshot_tests.rs`, append:

```rust
// ── End-to-end via WsBookUpdateProcessor ──────────────────────────────────────
//
// Uses a JSON payload with the exact shape from Polymarket's AsyncAPI docs
// example (https://docs.polymarket.com/asyncapi.json, `receiveBook` operation).

use polyfill2::{OrderBookManager, WsBookUpdateProcessor};

/// Parses the docs' example `book` payload and produces a book matching the
/// sent levels, with the best bid/ask at the correct end of the ladder.
#[test]
fn book_event_from_docs_example_parses_correctly() {
    let asset_id =
        "65818619657568813474341868652308942079804919287380422192892211131408793125422";

    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    let mut processor = WsBookUpdateProcessor::new(1024);

    let mut msg = format!(
        "{{\"event_type\":\"book\",\
          \"asset_id\":\"{asset_id}\",\
          \"market\":\"0xbd31dc8a20211944f6b70f31557f1001557b59905b7738480ca09bd4532f84af\",\
          \"bids\":[\
            {{\"price\":\"0.48\",\"size\":\"30\"}},\
            {{\"price\":\"0.49\",\"size\":\"20\"}},\
            {{\"price\":\"0.50\",\"size\":\"15\"}}\
          ],\
          \"asks\":[\
            {{\"price\":\"0.52\",\"size\":\"25\"}},\
            {{\"price\":\"0.53\",\"size\":\"60\"}},\
            {{\"price\":\"0.54\",\"size\":\"10\"}}\
          ],\
          \"timestamp\":\"1757908892351\",\
          \"hash\":\"0xabc123\"}}"
    )
    .into_bytes();

    let stats = processor.process_bytes(msg.as_mut_slice(), &manager).unwrap();
    assert_eq!(stats.book_messages, 1);
    assert_eq!(stats.book_levels_applied, 6);

    let snap = manager
        .with_book(asset_id, |b| b.snapshot())
        .expect("book exists");

    assert_eq!(snap.bids.len(), 3);
    assert_eq!(snap.asks.len(), 3);

    // Best bid = highest price = 0.50; best ask = lowest = 0.52.
    let bid_prices: Vec<_> = snap.bids.iter().map(|l| l.price).collect();
    let ask_prices: Vec<_> = snap.asks.iter().map(|l| l.price).collect();
    assert_eq!(bid_prices[0], dec("0.50"), "best bid should be first in snapshot (desc)");
    assert_eq!(ask_prices[0], dec("0.52"), "best ask should be first in snapshot (asc)");

    // Timestamp in 2020s, not ~57716.
    assert!((2020..2100).contains(&snap.timestamp.year()));
}

/// Alternating snapshots S1 -> S2 -> S1 through the WS decode+apply path
/// must produce the same book state as S1 on the third message, with S2's
/// levels fully gone.
#[test]
fn book_event_alternating_snapshots_no_state_leak() {
    let asset_id = "abc-stream-test";
    let manager = OrderBookManager::new(100);
    manager.get_or_create_book(asset_id).unwrap();

    let mut processor = WsBookUpdateProcessor::new(1024);

    let mk = |ts: u64, bids_json: &str, asks_json: &str| -> Vec<u8> {
        format!(
            "{{\"event_type\":\"book\",\"asset_id\":\"{asset_id}\",\"market\":\"0xabc\",\"timestamp\":{ts},\"bids\":{bids_json},\"asks\":{asks_json}}}"
        )
        .into_bytes()
    };

    // S1
    let mut s1 = mk(
        1,
        r#"[{"price":"0.74","size":"100"},{"price":"0.75","size":"200"}]"#,
        r#"[{"price":"0.76","size":"50"},{"price":"0.77","size":"30"}]"#,
    );
    processor.process_bytes(s1.as_mut_slice(), &manager).unwrap();

    // S2
    let mut s2 = mk(
        2,
        r#"[{"price":"0.60","size":"500"}]"#,
        r#"[{"price":"0.90","size":"10"}]"#,
    );
    processor.process_bytes(s2.as_mut_slice(), &manager).unwrap();

    let snap2 = manager.with_book(asset_id, |b| b.snapshot()).unwrap();
    assert_eq!(snap2.bids.len(), 1);
    assert_eq!(snap2.bids[0].price, dec("0.60"));
    assert_eq!(snap2.asks.len(), 1);
    assert_eq!(snap2.asks[0].price, dec("0.90"));

    // S1 again — S2's 0.60 / 0.90 must not leak.
    let mut s1_again = mk(
        3,
        r#"[{"price":"0.74","size":"100"},{"price":"0.75","size":"200"}]"#,
        r#"[{"price":"0.76","size":"50"},{"price":"0.77","size":"30"}]"#,
    );
    processor.process_bytes(s1_again.as_mut_slice(), &manager).unwrap();

    let snap3 = manager.with_book(asset_id, |b| b.snapshot()).unwrap();
    assert_eq!(snap3.bids.len(), 2);
    assert_eq!(snap3.asks.len(), 2);
    assert!(snap3.bids.iter().all(|l| l.price != dec("0.60")));
    assert!(snap3.asks.iter().all(|l| l.price != dec("0.90")));
}
```

- [ ] **Step 2: Verify `OrderBookManager::with_book` signature matches usage**

```bash
devenv shell -- rg "fn with_book" src/ | head -5
```

If the signature differs from `fn with_book<F, R>(&self, asset_id: &str, f: F) -> Option<R>` (returning `Option`) — e.g., if it returns `Result<R>` — adjust the `.expect("book exists")` / `.unwrap()` calls accordingly. Run the search first; update if needed before compiling.

- [ ] **Step 3: Run the test binary — all 10 tests (8 from Task 2 + 2 new E2E) pass**

```bash
devenv shell -- cargo test --all-features --test book_snapshot_tests 2>&1 | tail -20
```

Expected: `test result: ok. 10 passed; 0 failed`.

- [ ] **Step 4: Run full suite**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -10
```

Expected: all binaries green.

- [ ] **Step 5: Commit**

```bash
git add tests/book_snapshot_tests.rs
git commit -m "$(cat <<'EOF'
test(ws): end-to-end book snapshot tests using docs example

Two self-contained tests covering the full JSON decode + apply path:
a literal payload from Polymarket's AsyncAPI docs example (three bids
and three asks at documented prices) and an alternating S1-S2-S1
sequence that asserts no state leakage between snapshots through
the WsBookUpdateProcessor hot path.
EOF
)"
```

---

## Task 7: Bump version + CHANGELOG (Commit 6)

**Files:**
- Create: `CHANGELOG.md`
- Modify: `Cargo.toml` (version 0.2.0 → 0.3.0)

- [ ] **Step 1: Bump Cargo.toml version**

In `Cargo.toml`, find line 3:

```toml
version = "0.2.0"
```

Change to:

```toml
version = "0.3.0"
```

- [ ] **Step 2: Create CHANGELOG.md**

Create `CHANGELOG.md` at the repo root with this content:

```markdown
# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-04-23

### Breaking changes

- `OrderBook::apply_book_update` now replaces the book with the supplied
  levels instead of upserting them. Levels omitted from a `book` message
  are removed. This matches the actual wire contract of Polymarket CLOB V2
  `book` events. Pre-0.3.0 behavior left stale levels in the book and the
  local view drifted from exchange state over time. See issue #6.

### Fixed

- WebSocket `book` event is now correctly treated as a full snapshot
  rather than an incremental diff. Implemented via in-place mark-and-sweep
  on the `BTreeMap<Price, Qty>` values, preserving the project's zero-
  allocation contract on steady-state ladders. Fixes #6.
- WebSocket `book` event timestamp is now parsed as milliseconds instead
  of seconds. The `timestamp` field in Polymarket CLOB V2 `book` messages
  carries 13-digit millis; prior code interpreted it as Unix seconds,
  resolving a 2025 timestamp to year ~57,716. Fixes #6.

### Added

- `debug_assert!` in `OrderBook::apply_book_update` and
  `ws_hot_path::apply_levels` that validates ascending-price ordering on
  incoming `book` message level arrays. Compiled out in release; catches a
  server-side contract change loudly in dev/CI.

### Notes

- REST endpoint timestamps (`created_at`, `expiration`, `/book` timestamp
  in `decode.rs`) remain parsed as Unix seconds per the REST contract —
  only WS `book` event timestamps changed.
- Delta processing (`apply_delta_fast`, `price_change` events) is
  unchanged. Upstream polyfill-rs PR #24 removes delta support entirely;
  we do not follow that change because V2 `price_change` events are
  deltas and remain the common steady-state flow.

## [0.2.0] - 2026-04-22

- See git history for details (V2 migration fork of
  [`polyfill-rs`](https://github.com/floor-licker/polyfill-rs)).
```

- [ ] **Step 3: Verify the crate builds with the new version**

```bash
devenv shell -- cargo build --workspace 2>&1 | tail -5
devenv shell -- cargo package --allow-dirty --no-verify 2>&1 | tail -10
```

Expected: build succeeds; `cargo package` reports `Packaged N files, ...` with the new version `polyfill2-0.3.0.crate`.

- [ ] **Step 4: Run full test suite one more time**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml CHANGELOG.md
git commit -m "$(cat <<'EOF'
chore(release): bump to 0.3.0 + CHANGELOG

Breaking change: OrderBook::apply_book_update now replaces the book
instead of upserting supplied levels. Fixes both issue #6 bugs.
See CHANGELOG.md for details.
EOF
)"
```

---

## Task 8: Final verification

**Files:**
- None modified in this task.

- [ ] **Step 1: Full test suite, clean build, clippy clean**

```bash
devenv shell -- cargo test --all-features 2>&1 | tail -10
devenv shell -- cargo build --workspace 2>&1 | tail -5
devenv shell -- cargo clippy --all-features --all-targets -- -D warnings 2>&1 | tail -10
```

Expected: all three commands succeed with zero failures / zero warnings.

- [ ] **Step 2: Benchmark sanity check**

```bash
devenv shell -- cargo bench --bench book_updates -- --quick 2>&1 | tail -30
```

Expected: benches compile and run. The `--quick` flag uses a reduced sample size. If the benches report catastrophic regression (>3× slower on apply_book_update cases), investigate — might indicate an unexpected allocation pattern. Small regressions (<20%) are acceptable given the added mark/sweep overhead.

If `benches/book_updates.rs` does not exist or doesn't exercise snapshot paths, skip this step and note in the PR body.

- [ ] **Step 3: Verify commit log looks right**

```bash
git log --oneline main..HEAD
```

Expected: 6 commits in this order:
```
<sha> chore(release): bump to 0.3.0 + CHANGELOG
<sha> test(ws): end-to-end book snapshot tests using docs example
<sha> test(no-alloc): add zero-alloc assertions for steady-state snapshot replay
<sha> fix(book): treat ws book event as snapshot, not diff (mark-and-sweep)
<sha> fix(book): parse ws book timestamp as millis not seconds
<sha> test(book): add regression tests for ws book snapshot bugs
```

- [ ] **Step 4: Investigate V2 price_change.sequence vs snapshot timestamp**

This is the pre-existing concern flagged during brainstorming. `OrderBook::sequence` is written by both `apply_book_update` (as millis timestamp — now up to ~13 digits) and `apply_delta_fast` (as `delta.sequence`). If V2 `price_change.sequence` is a small counter (not millis), deltas arriving after a snapshot would be discarded as stale.

Find where `OrderDelta::sequence` is populated from V2 wire data:

```bash
devenv shell -- rg "OrderDelta \{|FastOrderDelta \{|\.sequence[^.]" src/ | head -30
```

Then inspect the price_change handler (likely in `src/stream.rs` or `src/decode.rs`) to see whether the incoming `price_change` carries a timestamp/sequence that's comparable to a millis timestamp or not.

**Possible outcomes:**

1. **`delta.sequence` is already set to the event's millis timestamp** — no issue, same scale as snapshot's `self.sequence`. Document in PR that we verified.
2. **`delta.sequence` is a small counter or uninitialized** — this is a pre-existing bug that our fix did not introduce but makes more visible. Document as a known limitation in the PR body and open a follow-up issue. Do NOT expand this PR's scope to fix it.

Either way, record what you found in the PR body.

---

## Task 9: Open the PR

**Files:**
- None modified.

- [ ] **Step 1: Push the branch**

```bash
git push -u origin fix/issue-6-ws-book-snapshot
```

- [ ] **Step 2: Create the PR**

```bash
gh pr create --title "fix: ws book snapshot semantics + millis timestamp (fixes #6)" --body "$(cat <<'EOF'
Fixes #6.

## Summary

Two bugs in `book` WebSocket message handling, both also present in upstream polyfill-rs (issue #23, unmerged PR #24):

1. **Timestamp parsed as seconds instead of millis.** Polymarket CLOB V2 `book` messages carry 13-digit millisecond timestamps; prior code used `DateTime::from_timestamp(ts, 0)` and resolved a 2025 timestamp to year ~57,716.
2. **`book` event treated as diff, not snapshot.** The server sends a full order-book snapshot on every `book` message. Prior code upserted the supplied levels without clearing the book, so stale levels from prior snapshots accumulated indefinitely.

## Design

Mark-and-sweep on `BTreeMap<Price, Qty>` values, in-place:

- **Mark** — walk `values_mut()` on both sides and zero each size (stack-only, no alloc).
- **Apply** — `insert(price, size)` for each incoming level (no alloc when key already exists; allocates only for genuinely new prices).
- **Sweep** — `retain(|_, v| *v != 0)` drops any level zeroed by phase 1 that phase 2 didn't overwrite (= vanished levels) plus any wire-zero levels. Then `trim_depth()`.

Steady-state replay (same price ladder, changed sizes) remains zero-alloc — the 6 existing no-alloc tests stay green unchanged.

Upstream PR #24's approach (`bids.clear()` + reinsert) was rejected because it deallocates every node on every message, breaking the zero-alloc contract this crate advertises.

Full design rationale + rejected alternatives: [`docs/superpowers/specs/2026-04-23-ws-book-snapshot-fix-design.md`](docs/superpowers/specs/2026-04-23-ws-book-snapshot-fix-design.md).

## Breaking change

`OrderBook::apply_book_update` semantics changed from upsert to full-snapshot replacement. Documented in CHANGELOG v0.3.0. Old behavior produced an incorrect book, so this is expected to affect nobody relying on documented behavior.

## Ordering debug_assert

Added `debug_assert!` in both `apply_book_update` and `ws_hot_path::apply_levels` that validates ascending-price ordering on incoming `bids` / `asks` arrays (which is what Polymarket's AsyncAPI docs example shows, though not contracted in spec). Compiled out in release; catches a server-side contract change loudly in dev/CI.

## Batch orders (sub-question from #6)

Already supported — no change needed:
- `Client::post_orders` — up to 15 signed orders per call
- `Client::create_and_post_orders` / `create_and_post_orders_with_type` — higher-level helpers
- `Client::cancel_orders` — batch cancel
- `BatchMidpointRequest`, `BatchPriceRequest`, `get_order_books_batch` — batch reads

## Test coverage

- `tests/book_snapshot_tests.rs` (new) — 10 regression tests: stale-level clearing, millis timestamp, zero-size dropping, max_depth enforcement, stale-sequence rejection, debug_assert panics (debug-only), alternating S1/S2/S1 state cleanliness, plus 2 E2E tests through `WsBookUpdateProcessor` using the docs' example payload.
- `tests/no_alloc_hot_paths.rs` (extended) — 4 new tests asserting zero allocations on steady-state snapshot replay through direct `apply_book_update`, same-ladder-different-sizes, `WsBookUpdateProcessor`, and `WebSocketStream` applier paths. All 6 pre-existing no-alloc assertions remain green.

## V2 `price_change.sequence` note

`OrderBook::sequence: u64` is written by both snapshots (as millis timestamp, now 13 digits) and deltas (as `delta.sequence`). [Investigation result goes here — see Task 8 Step 4.] This is a pre-existing concern not introduced by this PR; if problematic, will be tracked as a follow-up issue.

## Test plan

- [x] `cargo test --all-features` — full suite green
- [x] `cargo build --workspace` — clean
- [x] `cargo clippy --all-features --all-targets -- -D warnings` — clean
- [x] `cargo bench --bench book_updates -- --quick` — no catastrophic regression

## References

- Issue: #6
- Upstream issue: floor-licker/polyfill-rs#23
- Upstream PR (rejected approach): floor-licker/polyfill-rs#24
EOF
)"
```

- [ ] **Step 3: Record the PR URL**

The `gh pr create` command prints the PR URL. Note it — we'll need it for the post-merge issue comment.

---

## Self-Review

### Spec coverage check

- [x] Bug A fix — Tasks 3 (both call sites)
- [x] Bug B fix — Task 4 (hot path via begin/finish, non-hot path via apply_book_update)
- [x] Mark-and-sweep — Task 4
- [x] debug_assert on ordering — Task 4 (both sites)
- [x] `trim_depth` preserved — Task 4 (kept in `finish_ws_book_update`)
- [x] Existing no-alloc tests stay green — Task 4 Step 7 (hard-stop check)
- [x] New no-alloc tests — Task 5 (all 4 from spec's test plan)
- [x] Regression tests for both bugs — Task 2 (all 8 from spec's test plan)
- [x] E2E docs-example tests — Task 6 (both from spec's test plan)
- [x] `FastOrderDelta` / `OrderDelta` preserved — no task modifies them
- [x] REST `decode.rs` timestamps left alone — no task modifies them
- [x] Cargo.toml bumped to 0.3.0 — Task 7
- [x] CHANGELOG.md created — Task 7
- [x] PR body answers batch-orders sub-question — Task 9

Spec mentioned an optional proptest — intentionally deferred per spec ("optional"; extract only if file grows). Not in this plan.

### Placeholder scan

No "TBD", "TODO", "similar to", or "add appropriate error handling" phrases. Every step shows concrete code or an exact shell command with expected output.

One dynamic element: the PR body in Task 9 Step 2 contains `[Investigation result goes here — see Task 8 Step 4.]` — this is intentional: Task 8 Step 4 produces a finding that must be written into the PR body before pushing. That's a real dependency, not a placeholder.

### Type consistency check

- `OrderBookImpl::new(String, usize)` — used consistently in all test tasks.
- `BookUpdate { asset_id: String, market: String, timestamp: u64, bids: Vec<OrderSummary>, asks: Vec<OrderSummary>, hash: Option<_> }` — used consistently.
- `OrderSummary { price: Decimal, size: Decimal }` — consistent.
- `BTreeMap<Price, Qty>::values_mut()` / `.retain()` — consistent across Task 4 Steps 1, 2, 3.
- `WsBookUpdateProcessor::new(usize)` + `.process_bytes(&mut [u8], &OrderBookManager)` — consistent across Task 5 and Task 6.
- `OrderBookManager::with_book` — Task 6 Step 2 flags this for signature verification before compile; if it returns `Option<R>`, `.unwrap()` works; if `Result`, change to `?`/`.expect`.

### Ambiguity check

- "If `benches/book_updates.rs` does not exist or doesn't exercise snapshot paths, skip this step" (Task 8 Step 2) — explicit fallback, not ambiguity.
- Task 6 Step 2 signature verification step — guards against a discrepancy rather than leaving one.

All good.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-23-ws-book-snapshot-fix.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — Dispatch a fresh subagent per task, review between tasks. Best for a 9-task plan with a critical zero-alloc invariant check at Task 4 Step 7 that benefits from a clean-context reviewer.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

**Which approach?**
