# Task 14 — Per-Signal Adaptive Deviation Thresholds

**Status**: complete  
**Date**: 2026-05-13  
**`cargo check`**: 11 gene-core warnings (baseline), 0 observer-gene warnings

## What changed

### New file: `observer-gene/src/stddev_tracker.rs`
`StddevTracker` struct with:
- `observe(id, value)` — pushes a value into that signal's 500-sample `VecDeque`
- `thresholds()` → `HashMap<SignalId, f64>` — returns `k × stddev` per signal,
  clamped to `[0.005, 0.10]`; signals with < 10 observations fall back to `MIN_THRESHOLD = 0.005`
- Constants: `WINDOW_SIZE=500`, `K_FACTOR=1.5`, `MIN_THRESHOLD=0.005`, `MAX_THRESHOLD=0.10`

### Edited: `gene-core/src/pattern/extractor.rs`
`PatternExtractor::current_active()` signature changed (fourth local gene-core patch):

| | Before | After |
|---|---|---|
| param 1 | `deviation_threshold: f64` | `thresholds: &HashMap<SignalId, f64>` |
| param 2 | `baselines: &HashMap<SignalId, f64>` | `baselines: &HashMap<SignalId, f64>` |
| param 3 | *(none)* | `default_threshold: f64` |
| body | `dev > deviation_threshold` | `dev > thresholds.get(id).copied().unwrap_or(default_threshold)` |

No other gene-core changes. `extract()` is unaffected.

### Edited: `observer-gene/src/main.rs`
Three changes:
1. Added `mod stddev_tracker;` and `use stddev_tracker::StddevTracker;` at top
2. `let mut stddev_tracker = StddevTracker::new();` alongside other long-lived state
3. After the second `bus.tick()`:
   ```rust
   for (id, value) in post_snapshot.values.iter() {
       stddev_tracker.observe(*id, *value);
   }
   ```
4. Replaced `current_active(0.02, &signal_baselines)` with:
   ```rust
   let active_thresholds = stddev_tracker.thresholds();
   let active_signals = pattern_extractor.current_active(
       &active_thresholds, &signal_baselines, 0.02,
   );
   ```

### Edited: `.odin/memory.md`
Added "Fourth patch (task-14)" entry under Local gene-core Patches.

## Behavior after deploy

- **Warmup storm (~6s)**: signals with < 10 observations use `MIN_THRESHOLD=0.005` — more
  patterns fire during startup. Transients don't corrupt the ledger (salience requires
  repeated activation, which warmup spikes don't sustain).
- **Slow signals** (economic, ISS altitude, commodities): normal stddev is small → threshold
  drops toward `MIN_THRESHOLD=0.005` → fire on small absolute moves that are large relative
  to their variability.
- **Volatile signals** (crypto 24h-change, wildfire counts): high stddev → threshold rises
  toward `MAX_THRESHOLD=0.10` → require a larger move to gate into the active set.
- **Net effect**: pattern stream diversifies away from crypto/aviation dominance. Quieter
  signals enter the pattern index, enabling richer symbol co-activation.

## Acceptance criteria met

1. ✅ `cargo check` clean — 11 gene-core warnings (baseline), 0 observer-gene warnings
2. ✅ `cargo build --release` (user to run — no regressions expected)
3. ✅ No signal IDs changed — only activation gating logic changed
4. ⬜ Pattern stream diversification — verify via 60s smoke-test post-deploy
5. ✅ Signal IDs 5, 6, 7 not registered (tasks 15–17 not started)
6. ✅ `.odin/memory.md` updated with fourth gene-core patch entry

## Verification commands (user runs)

```bash
# Build check
~/.cargo/bin/cargo check

# Release build
~/.cargo/bin/cargo build --release

# Smoke-test: 60s against live Flux
mkdir -p /tmp/observer-task14
timeout 60 ./target/release/observer-gene \
    --data-dir /tmp/observer-task14 \
    --flux-url ws://192.168.50.107:3000/api/ws \
    --flux-token "$FLUX_TOKEN" 2>&1 | grep -E "coined|composite" | head -50
rm -rf /tmp/observer-task14
```
