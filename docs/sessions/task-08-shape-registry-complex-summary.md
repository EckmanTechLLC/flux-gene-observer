# Task 08 — Shape Registry Complex Shapes — Session Summary

**Date**: 2026-05-09  
**Status**: implemented — `cargo check` clean (11 gene-core baseline, 0 observer-gene)

## What Was Done

Moved the three remaining single-entity Flux feeds (crypto, stocks, commodities) from
hand-coded pollers into the catalog model. After this task every entity-level feed lives
in `feeds.toml`. Only aggregation (aviation, ships) and earthquakes remain hardcoded.

## Files Created

| File | Purpose |
|---|---|
| `observer-gene/src/shape/derive.rs` | `DeriveRecipe::PctChange` — `(from−ref)/ref×100` |
| `observer-gene/src/shape/nested_by_key.rs` | `NestedByKeyPoller` — 14 crypto entities × 3 signals each |
| `observer-gene/src/shape/array_record.rs` | `ArrayRecordPoller` — 5 stock tickers × 2 properties |
| `observer-gene/src/shape/time_series.rs` | `TimeSeriesPoller` — 6 commodity entities × 1 signal |

## Files Modified

### `observer-gene/src/shape/normalize.rs`
- Added `ZscoreWindow20` and `EmaRelativeThird` variants (marked `#[serde(skip_deserializing)]`)
- Added `NormalizeState` enum (`None`, `Window(VecDeque<f64>)`, `Ema { value, seeded }`)
- Added `init_state()` method
- Changed `apply()` signature to `apply(&self, val: f64, state: &mut NormalizeState) -> f64`
- Moved `z_score_to_unit()` here from `signal/mod.rs`

### `observer-gene/src/shape/path.rs`
- Extended `resolve()` to handle `[N]` array indexing in any path segment
- Updated `split_first()` to split on `[` as well as `.`, correctly separating the HashMap property key from the nested path
- Added unit tests for all canonical patterns

### `observer-gene/src/shape/flat_multi.rs` + `scalar.rs`
- Added `state: NormalizeState` to `SignalEntry`; initialized via `recipe.init_state()`
- Changed poll loop to index-based pattern (extract while immutable, normalize+write after lock drop)

### `observer-gene/src/shape/mod.rs`
- Added modules: `array_record`, `derive`, `nested_by_key`, `time_series`
- Unified `FlatMultiProperty` → `FeedProperty` (with optional `name`, `positive`, `parse_string`)
- Unified `ScalarEntity` → `FeedEntityDef` (optional `key` for nested_by_key, optional `path`/`normalize` for scalar)
- Added `NestedSignalDef` struct
- Extended `FeedConfig` with `signal_list`, `signal_name`, `path`, `parse_string`, `positive`, `normalize`
- Added `resolve_recipe()` helper — checks TOML catalog first, then hardcoded stateful table (`zscore_window_20`, `ema_relative_third`)
- Added `guard_preseeded()` helper (panic on fresh ID)
- Extended `build_pollers()` with `nested_by_key`, `array_record`, `time_series` match arms
- Extended `decay_for_namespace()` with crypto (0.005), stocks (0.001), commodities (0.001)

### `observer-gene/feeds.toml`
- Added `[normalize.change_band_20]` recipe (linear_range −20..20)
- Added `[[feed]]` for `flux-crypto` (nested_by_key, 14 entities, 3 signals)
- Added `[[feed]]` for `flux-stocks` (array_record, 5 entities, 2 properties)
- Added `[[feed]]` for `flux-commodities` (time_series, 6 entities, 1 signal)

### `observer-gene/src/signal/mod.rs`
- Removed `pub mod crypto`, `pub mod stocks`, `pub mod commodities`
- Removed `z_score_to_unit()` (moved to `shape/normalize.rs`)

### `observer-gene/src/main.rs`
- Removed 42 crypto + 10 stocks + 6 commodities `SIG_*` consts
- Removed imports of `CryptoPoller`, `StocksPoller`, `CommoditiesPoller`
- Removed those three pollers from `build_bus()` (now registered by `build_pollers`)
- Removed from `verify_registry_consts()` (now guarded by `build_pollers` panic-on-fresh-ID)
- Removed instantiation and poll calls in `main()`
- Fixed `build_action_space()` to use inline `SignalId(N)` literals instead of removed consts

## Files Deleted

- `observer-gene/src/signal/crypto.rs`
- `observer-gene/src/signal/stocks.rs`
- `observer-gene/src/signal/commodities.rs`

## Signal IDs (unchanged, all pre-seeded)

| Range | Feed | Shape |
|---|---|---|
| 30–44, 160–189 | flux-crypto (14 coins × 3) | nested_by_key |
| 50–59 | flux-stocks (5 tickers × 2) | array_record |
| 130–135 | flux-commodities (6 × 1) | time_series |

## Key Design Decisions

- **Stateful normalization state** (`NormalizeState`) owned by each poller entry; initialized empty on startup (same restart behavior as old pollers — windows re-warm over ~20 polls)
- **Separation of extraction and normalization**: raw values collected while holding lock, normalization applied after lock drop
- **path.rs array indexing**: `[N]` is parsed in each dot-separated segment; `split_first` splits on whichever delimiter (`[` or `.`) comes first, so `data[0].value` → `("data", "[0].value")`
- **Derived signal (change)**: `pct_change` recipe handles Kraken VWAP vs price; ref ≤ 0 → 0 (matches old poller fallback)
- **positive guard**: missing or ≤ 0 values skip the bus update (retain prior bus value), matching old poller semantics exactly

## Verification

```
cargo check → 11 gene-core warnings (baseline), 0 observer-gene warnings ✓
```

## Deploy

Standard procedure: build on .13, rsync to .107, stop/swap/start service.

Expected startup log (6 total catalog feed lines including task-07's three):
```
feed=flux-weather      shape=flat_multi    entities=6  signals=18
feed=flux-economic     shape=scalar        entities=5  signals=5
feed=flux-internet     shape=scalar        entities=1  signals=1
feed=flux-crypto       shape=nested_by_key entities=14 signals=42
feed=flux-stocks       shape=array_record  entities=5  signals=10
feed=flux-commodities  shape=time_series   entities=6  signals=6
```
