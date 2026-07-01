# Task 16: SIG_LEDGER_DEGENERACY — Symbol Diversification Meta-Signal

**Status**: implemented, awaiting deploy (bundled in task-18)
**Date**: 2026-05-13

## What was done

Added SIG_LEDGER_DEGENERACY (ID 6, weight 3.0) — a Shannon-entropy-based signal that
measures how concentrated recent symbol activation is. High value (near 1.0) means one
or a few symbols dominate all activation; low value (near 0.0) means activation spreads
across many symbols evenly.

## Files created / edited

| File | Operation |
|------|-----------|
| `observer-gene/src/degeneracy_tracker.rs` | **created** — `DegeneracyTracker` struct + impl |
| `observer-gene/src/main.rs` | **edited** — mod/use, const, verify table entry, bus registration, instantiation, tick-loop wiring |

## Changes in main.rs

1. **`mod degeneracy_tracker` + `use DegeneracyTracker`** — alongside other tracker mods
2. **`const SIG_LEDGER_DEGENERACY: SignalId = SignalId(6)`** — alongside SIG_META/SIG_DRIVE/SIG_FAMILIARITY
3. **`verify_registry_consts` table** — added `"internal/ledger_degeneracy#value" → SIG_LEDGER_DEGENERACY.0`
4. **`build_bus()`** — `bus.register_with_id(SIG_LEDGER_DEGENERACY, SignalClass::Derived, 0.0, 0.02, 3.0)`
5. **Instantiation** — `let mut degeneracy_tracker = DegeneracyTracker::new();`
6. **Tick loop** — after `SymbolActivationFrame::build`, before `composition_engine.observe`:
   ```rust
   let active_indices: Vec<u32> = frame.active.iter().map(|(idx, _, _)| *idx).collect();
   degeneracy_tracker.observe(active_indices);
   let degeneracy = degeneracy_tracker.degeneracy();
   bus.set_value(SIG_LEDGER_DEGENERACY, degeneracy);
   ```

## DegeneracyTracker design

- 1000-tick rolling window of per-tick active symbol index lists
- Counts occurrences of each distinct symbol across the window
- Shannon entropy over those counts, normalized by `ln(N_distinct_symbols)`
- Returns `1 - H_norm` clamped to `[0, 1]`
- Edge case: ≤1 distinct symbol → returns 1.0 (maximally degenerate)
- Edge case: empty history → also returns 1.0 via the ≤1 branch

## Signal registration

- Class: `Derived`
- Baseline: 0.0 (zero degeneracy is the ideal)
- Decay: 0.02 (slower than familiarity — degeneracy is slower-changing)
- Weight: 3.0 (per ADR 002)
- Pre-seeded ID: 6 (reserved by task-15 in `seed-registry.json`)

## Verification

`cargo check` result: **11 gene-core warnings (baseline), 0 observer-gene warnings** ✅

## Expected behavior post-deploy

- **Startup**: degeneracy = 1.0 (empty history, ≤1 symbol branch). Settles within ~1000 ticks (~1s)
- **Initial production reading**: with Φ_0231/0330 dominance observed, expect 0.8–0.95
- **Over 3–7 days**: should trend toward 0.3–0.5 as gene exercises AdjustBaseline on quiet
  signals and CoinDerivedSignal to diversify activation patterns
- **Audit log at startup**: `seeded=1 fresh=0` for `internal/ledger_degeneracy#value` —
  confirming pre-seeded slot resolved correctly at ID 6

## Bus signal count

Post-task-16: **31 signals** in `build_bus()` (was 30 after task-15)
- 0–2: continuity (3)
- 3–4: meta, drive (2)
- 6: ledger_degeneracy (1) ← new
- 7: familiarity (1)
- 100–103: earthquakes (4)
- + all catalog-driven signals registered by `shape::build_pollers()` at runtime
