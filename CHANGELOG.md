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
  resolving a 2025 timestamp to year ~57,716.

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

## [0.2.0] - 2026-04-23

- See git history for details (V2 migration fork of
  [`polyfill-rs`](https://github.com/floor-licker/polyfill-rs)).
