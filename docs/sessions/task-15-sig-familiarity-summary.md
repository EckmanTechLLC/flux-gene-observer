# task-15-sig-familiarity — Session Summary

**Date**: 2026-05-13
**Status**: implemented, awaiting deploy (bundled with tasks 16–17a in task-18)

## What Was Done

Added SIG_FAMILIARITY (ID 7, weight 2.0) — the anti-stagnation meta-signal for ADR 002.
Also reserved seed slots for IDs 5 and 6 (prediction_error and ledger_degeneracy,
to be registered in tasks 17 and 16 respectively).

## Files Changed

### Created
- `observer-gene/src/familiarity_tracker.rs` — `FamiliarityTracker` struct

  Rolling 500-tick window of pattern_ids. `pattern_id()` hashes sorted active-signal
  IDs (order-independent; uses DefaultHasher, not persistent across restarts). `observe()`
  pushes to VecDeque and trims. `familiarity()` returns count/window_size ∈ [0.0, 1.0].

### Edited
- `observer-gene/seed-registry.json` — Added 3 new `by_key` entries:
  - `"internal/familiarity#value": 7` — registered this task
  - `"internal/ledger_degeneracy#value": 6` — slot reserved for task-16
  - `"internal/prediction_error#value": 5` — slot reserved for task-17
  - `high_water_mark` unchanged at 189

- `observer-gene/src/main.rs` — 6 changes:
  1. `mod familiarity_tracker;` + `use familiarity_tracker::FamiliarityTracker;`
  2. `const SIG_FAMILIARITY: SignalId = SignalId(7);` (after SIG_DRIVE)
  3. `verify_registry_consts`: added `("internal/familiarity#value", SIG_FAMILIARITY.0)`
  4. `build_bus()`: `bus.register_with_id(SIG_FAMILIARITY, SignalClass::Derived, 0.0, 0.05, 2.0)`
  5. Initialization: `let mut familiarity_tracker = FamiliarityTracker::new();`
  6. Tick loop: 4-line block after `pattern_extractor.current_active(...)`:
     ```rust
     let fam_pattern_id = FamiliarityTracker::pattern_id(&active_signals);
     familiarity_tracker.observe(fam_pattern_id);
     let fam = familiarity_tracker.familiarity(fam_pattern_id);
     bus.set_value(SIG_FAMILIARITY, fam);
     ```

## Build Check

```
cargo check → 11 gene-core warnings (baseline), 0 observer-gene warnings
```

## Signal Count After This Task

- Pre-task-14: 26 signals (before ADR 002)
- task-14: no new signals (adaptive thresholds only)
- **task-15**: +1 → **27 signals** (SIG_FAMILIARITY on bus)
- task-16 will add SIG_LEDGER_DEGENERACY (ID 6) → 28
- task-17 will add SIG_PREDICTION_ERROR (ID 5) → 29
  (Note: ADR 002 target was 32; bus count also includes catalog-driven world signals)

## Design Notes

- `familiarity_tracker.rs` rolls its own hash (sorted SignalId vec → DefaultHasher) rather
  than calling `compute_pattern_hash` (private in gene-core). Semantics match: stable
  hash of sorted signal IDs; magnitudes intentionally ignored.
- First tick: history empty → 0.0. Second tick: 1 item in window → familiarity = 1.0
  briefly, decays as window fills with other patterns. Initialization noise — harmless.
- DefaultHasher is not portable across restarts; that's intentional. History rebuilt fresh
  each start gives a "novelty burst" after restart.
- `high_water_mark=189` unchanged — IDs 5, 6, 7 are pre-seeded, not freshly allocated
  above the mark. Tasks 16 and 17 will register against pre-seeded slots (audit log
  will show `seeded=1 fresh=0` for each).
