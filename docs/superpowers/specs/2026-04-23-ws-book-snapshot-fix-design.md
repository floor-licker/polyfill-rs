# WS `book` snapshot + millis timestamp fix

**Date:** 2026-04-23
**Issue:** [#6](https://github.com/onsails/polyfill2-rs/issues/6)
**Upstream reference:** floor-licker/polyfill-rs [#23](https://github.com/floor-licker/polyfill-rs/issues/23), unmerged [PR #24](https://github.com/floor-licker/polyfill-rs/pull/24)
**Target release:** polyfill2 v0.3.0 (minor bump — semantic change to public API)

## Problem

Two bugs in the WebSocket `book` message processing path:

**Bug A — timestamp parsed as seconds, not milliseconds.**
Polymarket CLOB WSS `book` events carry a 13-digit millisecond timestamp. `src/book.rs:417` (`begin_ws_book_update`) and `src/book.rs:474` (`apply_book_update`) call `DateTime::from_timestamp(ts as i64, 0)`, interpreting the value as seconds. A 2025-era timestamp resolves to year ~57,716.

**Bug B — `book` event treated as a diff, not a snapshot.**
The server sends a full order-book snapshot on every `book` message. Current code upserts the supplied levels without clearing the book, so stale levels from earlier snapshots are never removed. The local book diverges from the exchange state over time.

Both bugs are also present in upstream polyfill-rs and are the subject of upstream issue #23.

## Non-goals

- Not touching REST timestamp parsing in `src/decode.rs` — REST endpoints send Unix seconds and those sites are correct.
- Not removing `apply_delta_fast` / `FastOrderDelta` / `OrderDelta` — required for V2 `price_change` (delta) events, which are the common case in practice. Upstream PR #24 removes these; we don't follow.
- Not switching to upstream PR #24's `.skip(len - max_depth)` optimization — undocumented server ordering contract, and their math is wrong for asks anyway (they skip best asks; fixing would require per-side split). `trim_depth()` is robust and order-agnostic.
- Not adding new public methods or deprecation shims — pre-1.0, clean semantic swap is less confusing than a duplicated API.

## Approach: mark-and-sweep snapshot replacement

The snapshot must replace the book but we want to preserve the project's zero-allocation contract for the hot path. `BTreeMap::clear()` deallocates every node, so naive clear-and-reinsert allocates on every message.

Three-phase mark-and-sweep using in-place value mutation:

1. **Mark** — walk `values_mut()` on both sides and set each size to sentinel `0`. Stack-only iteration; no heap touch. O(n) writes, n ≤ `max_depth`.
2. **Apply** — for each level in the incoming message, update the map at that price:
   - Hot path (`apply_ws_book_level_fast`): existing helper `apply_bid_delta_fast`/`apply_ask_delta_fast` already short-circuits on size==0 (removes the node), otherwise `insert(price, size)` overwrites in place (no alloc) or allocates a node for a new price. Behavior unchanged.
   - Non-hot path (`apply_book_update`): unconditional `insert(price, size)`. Wire size==0 is carried through to phase 3 for removal. Simpler; negligible alloc cost since this path is not latency-sensitive.
3. **Sweep** — `retain(|_, v| *v != 0)` on both sides. Drops any price that phase 1 zeroed and phase 2 didn't overwrite (= vanished from the snapshot) and any wire-size-0 carried through from phase 2. Followed by `trim_depth()` to enforce `max_depth`.

### Why this preserves zero-alloc

| Scenario | Allocations |
|---|---|
| Replay identical snapshot | 0 |
| Same price ladder, different sizes (steady state) | 0 |
| Some levels disappeared | 0 allocs, some deallocs |
| A new price appeared | 1 node alloc per new price |
| Level count exceeded `max_depth` | `trim_depth` deallocates excess |

The existing no-alloc tests in `tests/no_alloc_hot_paths.rs` exercise the "same ladder, different sizes" case. They stay green unchanged.

### Cost analysis

Per `book` message: O(n + m) where n = current book levels (≤ `max_depth`, default 100) and m = incoming snapshot levels. Absolute budget: ~1–5 μs per message including pre/post passes. Polymarket WSS book-message rate is low (snapshots are rare; `price_change` deltas are the steady-state flow), so aggregate overhead is negligible.

**Lower bound:** any correct snapshot implementation must touch every existing entry at least once to detect disappearances. O(n) is unavoidable. Mark-and-sweep uses two passes (mark + sweep) instead of the theoretical minimum one (sequence tagging), but sequence tagging would change the `BTreeMap` value type and ripple through ~30+ reader call sites. Not worth the blast radius for a ~2× overhead on an already-cheap path.

### Rejected alternatives

1. **Naive `bids.clear()` + re-insert** (upstream PR #24 approach). Simpler code but deallocates every node every message. Breaks every zero-alloc test. Rejected.
2. **Generation / sequence tagging**: value becomes `(u64 gen, Qty size)`. 2× per-value memory, requires destructuring at every reader (`best_bid_fast`, `spread_fast`, `mid_price_fast`, `snapshot`, iteration, market-impact calc, etc.). Too invasive for a bug fix. Rejected.
3. **Dual-buffer swap**: two `BTreeMap`s per side, swap on finish. Clearing the back buffer still deallocs nodes — same issue as naive clear. Rejected.
4. **Ordered skip** (`skip(len - max_depth)` per upstream PR #24): relies on undocumented server ordering; Polymarket V2 docs do not contract array order. Upstream's math is also asymmetric-wrong: bids ascending means `skip` drops worst (correct), but asks ascending means `skip` drops best (incorrect — should be `take`). Rejected in favor of input-order-agnostic `trim_depth()`.

## Architecture

Two files changed:

```
src/book.rs
├── OrderBook::begin_ws_book_update      (hot path)
│     - Fix timestamp: from_timestamp → from_timestamp_millis
│     - Add mark phase: values_mut → 0 on both sides
│
├── OrderBook::apply_ws_book_level_fast  (hot path)
│     - No change (insert is already upsert-safe; zero-sized are tolerated)
│
├── OrderBook::finish_ws_book_update     (hot path)
│     - Add sweep phase: retain(|_, v| *v != 0) on both sides
│     - Keep trim_depth() call
│
├── OrderBook::apply_book_update         (non-hot path; REST, tests, WebSocketStream)
│     - Fix timestamp: from_timestamp → from_timestamp_millis
│     - Replace upsert loops with mark + insert + sweep + trim_depth
│     - Drop `if size == 0 { remove } else { insert }` branch — insert all, sweep zeros
│     - Add debug_assert! on input bids/asks ascending-price ordering
│
└── trim_depth()                         (unchanged — robust to any input order)

src/ws_hot_path.rs
└── apply_levels
      - Add rolling debug_assert! on input ordering (prev_price <= current_price)
```

No change to `OrderBookManager` forwarder, `FastOrderDelta`, `OrderDelta`, delta-apply paths, REST decoders, or any public type shape.

## Public API change

Semantic change (no signature change) on one public method:

```rust
impl OrderBook {
    /// Apply a full order-book snapshot from a WebSocket `book` event.
    ///
    /// **Semantics changed in 0.3.0:** previously upserted the supplied levels
    /// (preserved any levels omitted from the message). Now replaces the book:
    /// levels omitted from the message are removed. Matches the actual wire
    /// contract of Polymarket CLOB V2 `book` messages. See issue #6.
    pub fn apply_book_update(&mut self, update: &BookUpdate) -> Result<()>;
}
```

`OrderBookManager::apply_book_update` forwards to `OrderBook::apply_book_update`; its semantics follow transparently.

## Ordering debug_assert

Polymarket V2 docs do not contract array order. The example payload in `asyncapi.json` shows **ascending price on both sides** (`bids: [0.48, 0.49, 0.50]`, `asks: [0.52, 0.53, 0.54]`). We assert this in debug builds to catch a server-side ordering change early:

- In `apply_book_update`: `debug_assert!(update.bids.windows(2).all(|w| w[0].price <= w[1].price))` and symmetric for asks.
- In `ws_hot_path.rs::apply_levels`: rolling `debug_assert!(current_price >= prev_price)` as levels are decoded.

`debug_assert!` compiles out under `--release` (no perf/alloc impact on hot path). The `test` profile in `Cargo.toml` keeps debug_assertions on, so CI catches any regression.

## Test plan

### `tests/book_snapshot_tests.rs` (new file)

```rust
snapshot_clears_stale_levels                         // Bug B regression
snapshot_timestamp_parses_as_millis                  // Bug A regression
snapshot_drops_zero_sized_levels
snapshot_enforces_max_depth_keeping_best
snapshot_ignored_when_timestamp_le_sequence
snapshot_panics_on_descending_bids_in_debug          // debug_assertions only, #[should_panic]
snapshot_panics_on_descending_asks_in_debug          // debug_assertions only, #[should_panic]
snapshot_alternating_s1_s2_s1_has_no_leakage        // round-trip state cleanliness
```

### `tests/no_alloc_hot_paths.rs` (extend)

All 6 existing tests must continue to assert zero allocations (non-negotiable — this is the project's core contract).

New tests:
```rust
no_alloc_steady_state_snapshot_replay                // warm with S, replay S → 0 allocs
no_alloc_same_ladder_different_sizes                 // {75,76} warm, apply {75,76 new sizes} → 0 allocs
no_alloc_same_ladder_via_ws_processor                // same via WsBookUpdateProcessor
no_alloc_same_ladder_via_ws_applier                  // same via WebSocketStream book applier
```

### `tests/ws_integration_tests.rs` (extend)

```rust
book_event_from_docs_example_parses_correctly        // literal JSON from Polymarket AsyncAPI docs
book_event_alternating_snapshots_no_state_leak       // S1 → S2 → S1, verify each matches
```

### `src/book.rs` inline `#[cfg(test)]` (optional property test)

```rust
proptest! {
    #[test]
    fn any_snapshot_matches_input_after_apply(
        bids in prop::collection::vec(arbitrary_level(), 0..50),
        asks in prop::collection::vec(arbitrary_level(), 0..50),
    ) {
        // normalize input: sort ascending, dedup by price, drop zero-size, trim to max_depth
        // apply → snapshot → assert equal
    }
}
```

Extract to its own file if size pushes `src/book.rs` >50% tests (per CLAUDE-base rule).

### Verification

```bash
devenv shell -- cargo test --all-features --test book_snapshot_tests
devenv shell -- cargo test --all-features --test no_alloc_hot_paths
devenv shell -- cargo test --all-features --test ws_integration_tests
devenv shell -- cargo test --all-features
devenv shell -- cargo build --workspace
devenv shell -- cargo clippy --all-features --all-targets -- -D warnings
```

## Rollout

### Branch: `fix/issue-6-ws-book-snapshot`

Commit sequence (each commit compiles + passes at that point):

1. `test(book): add failing regression tests for ws book snapshot bugs` — TDD baseline for both bugs.
2. `fix(book): parse ws book timestamp as millis not seconds` — trivial two-site fix; bug-A tests go green.
3. `fix(book): treat ws book event as snapshot, not diff (mark-and-sweep)` — core change; bug-B tests go green; all existing no-alloc tests still green.
4. `test(no-alloc): add zero-alloc assertions for steady-state snapshot replay` — the new no-alloc guarantees.
5. `test(ws): end-to-end book snapshot tests using docs example` — E2E coverage.
6. `chore(release): bump to 0.3.0 + CHANGELOG` — version bump; add new `CHANGELOG.md` (none exists today) with a "Breaking changes" section documenting the `apply_book_update` semantic change and a "Fixed" section for both bugs.

### PR

Fixes #6. Body links upstream #23/#24 for context, summarizes both bugs, calls out the `apply_book_update` semantic change as breaking, lists test coverage, and answers the batch-orders sub-question by pointing at `Client::post_orders` / `create_and_post_orders` / `cancel_orders` / `BatchMidpointRequest` / `BatchPriceRequest` / `get_order_books_batch`.

### Release

After merge: tag `v0.3.0`, push tag, release workflow validates + publishes to crates.io. Verify via crates.io API.

### Issue reply

After release: one comment confirming fix, listing batch-order endpoints, then close the issue.

## Risks

| Risk | Mitigation |
|---|---|
| Mark-and-sweep slower than naive clear in pathological cases | O(n ≤ max_depth) trivial; `benches/book_updates.rs` catches any regression |
| `debug_assert!` fires on real server traffic we haven't observed | Loud in dev/CI, silent in release; first hit is a GitHub issue, not a production crash |
| Users depending on old upsert semantics | Unlikely (old behavior produced incorrect book); documented breaking in CHANGELOG; 0.3.0 signals it |
| Non-steady-state market with constant ladder churn | Acceptable: alloc cost is ~1 BTreeMap node per new price, same as current delta-insert path |
