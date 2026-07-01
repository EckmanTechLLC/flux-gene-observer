# Task 17: SIG_PREDICTION_ERROR — Summary

**Status**: implemented, awaiting deploy (bundled in task-18)
**Date**: 2026-05-13

## What was done

Added `SIG_PREDICTION_ERROR` (ID 5, weight 5.0) — the third and heaviest ADR 002
meta-signal. Closes the perception-action learning loop: AdjustDecay actions now have
a causal signal they can measurably affect.

## Files created

### `observer-gene/src/predict.rs` (new, ~110 lines)

Two structs:

- `Predictor` — per-signal EMA value + trend tracker with one pending forecast.
  - `observe(value)` — updates `ema_value` (α=0.05) and `ema_trend` (α=0.01)
  - `predict_at(horizon)` — linear extrapolation: `ema_value + horizon × ema_trend`

- `PredictionEngine` — aggregates all per-signal predictors.
  - `tick(current_tick, snapshot)` — for each signal: observe → score matured
    prediction (push squared error to 100-entry rolling window) → emit new prediction
    at `current_tick + 5000`
  - `aggregate_error()` — `mean( tanh( avg_sq_err / 0.1 ) )` across all windows;
    returns 0.0 until first predictions mature (~5000 ticks)

Memory footprint: ~30 bytes/predictor × 884 = ~27 KB; ~700 KB for error windows.

## Files edited

### `observer-gene/src/main.rs`

Four changes:

1. **`mod predict` + `use predict::PredictionEngine`** — module declaration and import
   alongside other trackers (degeneracy_tracker, familiarity_tracker, stddev_tracker).

2. **`SIG_PREDICTION_ERROR: SignalId = SignalId(5)`** const added in the derived-internal
   block alongside SIG_META (3), SIG_DRIVE (4), SIG_LEDGER_DEGENERACY (6), SIG_FAMILIARITY (7).

3. **`verify_registry_consts`** — entry `("internal/prediction_error#value", SIG_PREDICTION_ERROR.0)`
   added. Pre-seeded in task-15's seed-registry.json; resolves to ID 5 at startup.

4. **`build_bus()`** — `bus.register_with_id(SIG_PREDICTION_ERROR, SignalClass::Derived, 0.0, 0.01, 5.0)`
   added between SIG_DRIVE and SIG_LEDGER_DEGENERACY.

5. **Instantiation** — `let mut prediction_engine = PredictionEngine::new()` alongside
   other trackers.

6. **Tick-loop wiring** — after the `stddev_tracker.observe` loop, before `imbalance_history`:
   ```rust
   prediction_engine.tick(tick, &post_snapshot.values);
   let pred_err = prediction_engine.aggregate_error();
   bus.set_value(SIG_PREDICTION_ERROR, pred_err);
   ```

## Verification

```
~/.cargo/bin/cargo check
```

Result: 11 gene-core warnings (baseline unchanged), **0 observer-gene warnings**.

## Design notes

- **One pending prediction per signal** (not a queue) — caps memory at ~27 KB for
  predictors regardless of signal count or horizon length.
- **Stale-eviction**: if a signal stops publishing, its pending prediction gets
  resolved-and-cleared on the next observation tick (when `target <= current_tick`).
  No prediction window accumulation for silent signals.
- **First 5000 ticks**: `aggregate_error()` returns 0.0 (no matured predictions yet).
  Bus value ramps up as first predictions mature at tick ~5000.
- **Constant signals**: zero trend → zero error contribution → 0.0 per signal.
  These don't inflate the aggregate.
- **EMA parameters** (α_value=0.05, α_trend=0.01): chosen per ADR 002. Trend lags
  value by ~20× which stabilizes extrapolation. If error stays high post-deploy,
  try α_value=0.1 or shorter HORIZON_TICKS.

## Expected behavior

| Phase | SIG_PREDICTION_ERROR value |
|-------|--------------------------|
| Ticks 0–5000 | 0.0 (no matured predictions) |
| Ticks 5000–50000 | 0.1–0.7 (windows filling, noisy) |
| Day 1–7 post-deploy | Trending down as AdjustDecay actions tune decay rates |

Target: 0.4–0.7 day 0 → 0.1–0.3 day 7 as regulation learns which decay settings
minimize prediction error.

## ADR 002 status after this task

All four code tasks complete:
- task-14: per-signal adaptive deviation thresholds ✅
- task-15: SIG_FAMILIARITY (ID 7, weight 2.0) ✅
- task-16: SIG_LEDGER_DEGENERACY (ID 6, weight 3.0) ✅
- task-17: SIG_PREDICTION_ERROR (ID 5, weight 5.0) ✅
- task-17a: extend signal_key() ✅

Next: task-18 bundles and deploys all of the above as a single binary swap to .107.
