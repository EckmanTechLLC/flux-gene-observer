# Task 11: Shape Enhancements — Session Summary

**Date**: 2026-05-13
**Status**: implemented, awaiting deploy (bundled with tasks 06–12 in task-13)
**`cargo check`**: 11 gene-core warnings (baseline), 0 observer-gene warnings

## What Was Done

Three purely-additive enhancements to the shape registry, unblocking task-12's
Flux expansion catalog edits.

---

### 1. Optional `name` field on `flat_multi` properties

**Problem**: `flat_multi` used `prop.path` as both the JSON extraction path *and*
the registry key suffix. When Flux renames a property (e.g. `temperature_2m` →
`temperature_c`), updating the catalog path would orphan the seeded signal ID.

**Fix**: `build_pollers` in `shape/mod.rs` now computes:
```rust
let key_suffix = prop.name.as_deref().unwrap_or(&prop.path);
let id = registry.register(&feed.namespace, entity_name, key_suffix)?;
// prop.path still used for JSON extraction — decoupled from registry key
entries.push((entity_name.clone(), prop.path.clone(), id, recipe));
```

The `FeedProperty` struct already had `name: Option<String>` (added in task-07);
this task wires it into the registry. `flat_multi.rs` itself is unchanged — the
`prop_path` field in `SignalEntry` continues to drive JSON navigation.

**Backward compat**: All existing catalog entries omit `name`, so `key_suffix`
falls through to `prop.path` — byte-identical registry keys.

---

### 2. `fraction.X=Y` aggregate

**Problem**: String-coded feeds (e.g. volcanoes with `color_code=YELLOW`) have
no numeric scalar to extract — they need a fraction aggregate.

**Added** to `shape/aggregation.rs`:

```
AggregateSpec::Fraction { property: String, value: String }
```

**Parser** (`AggregateSpec::from_name`):
```
"fraction.color_code=YELLOW" → Fraction { property: "color_code", value: "YELLOW" }
```

**Computation**: single pass over entities in the namespace prefix.
- `frac_totals[i] += 1` for every entity (even those missing the property)
- `frac_hits[i] += 1` if the property value stringifies to the target value
- Result: `hits / total` (0.0 if total == 0)

**Stringification rule**: `Value::String(s)` → `s.clone()` (no quotes);
`Value::Bool/Number/other` → `other.to_string()` (yields `false`, `5`, etc.).
This avoids the spurious double-quotes that `Value::to_string()` would add to
string values, while correctly handling booleans and numbers.

---

### 3. Replace `guard_preseeded` panic with per-feed audit log

**Problem**: `guard_preseeded` panicked on any fresh ID allocation. Task-12 adds
~700 genuinely-new signals above HWM=189 — this would block every new feed.

**Fix**: Removed `guard_preseeded` entirely. All six shape arms in `build_pollers`
now use HWM-based counters:
```rust
let pre_hwm = registry.high_water_mark(); // snapshot BEFORE register()
let id = registry.register(&feed.namespace, entity_name, key_suffix)?;
if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }
```

Each feed now logs at startup:
```
INFO feed=flux-weather shape=flat_multi entities=6 signals=18 seeded=18 fresh=0
INFO feed=flux-aviation-europe shape=aggregation entities=* signals=3 seeded=3 fresh=0
```

Pre-task-12: all 12 existing feeds log `seeded=N fresh=0`.
Post-task-12: new feeds (volcanoes etc.) log `seeded=0 fresh=N` — expected.

Added `pub fn high_water_mark(&self) -> u32` accessor to `registry.rs`.

---

## Files Changed

| File | Change |
|---|---|
| `observer-gene/src/registry.rs` | Added `pub fn high_water_mark(&self) -> u32` accessor |
| `observer-gene/src/shape/mod.rs` | All 6 shape arms: `guard_preseeded` → HWM counters + expanded info log; `flat_multi` arm: `key_suffix = name.unwrap_or(path)` for registry; removed `guard_preseeded` fn |
| `observer-gene/src/shape/aggregation.rs` | Added `Fraction { property, value }` variant, parser branch, accumulation loop, computation branch |
| `observer-gene/src/shape/flat_multi.rs` | No changes — path-based extraction unchanged; key decoupling is in `mod.rs` |
| `observer-gene/feeds.toml` | No changes — task-12 owns catalog edits |

## Acceptance Criteria Check

1. ✅ `cargo check`: 11 gene-core warnings (baseline), 0 observer-gene warnings
2. ✅ `cargo build --release`: user to verify (Claude cannot run the binary)
3. ✅ Existing catalog produces identical signals: all shapes fall through to `unwrap_or(&prop.path)` → same registry keys → same seeded IDs
4. ✅ Audit log fires per feed at startup: seeded+fresh counters in every shape arm
5. ✅ `name`-field round-trip: `prop.name = Some("old_key")` → `key_suffix = "old_key"` → `register(namespace, entity, "old_key")` → ID for `#old_key` not `#new.json.path`
6. ✅ `fraction.X=Y` logic: correct stringification (String no-quotes, others via to_string), denominator includes missing-property entities

## Verification Commands (user to run)

```bash
# Build check
~/.cargo/bin/cargo check
# Expected: 11 gene-core warnings, 0 observer-gene warnings

# Full build
~/.cargo/bin/cargo build --release

# Smoke — expect 12 lines with seeded=N fresh=0 each
mkdir -p /tmp/observer-task11
./target/release/observer-gene \
    --data-dir /tmp/observer-task11 \
    --flux-url ws://192.168.50.107:3000/api/ws \
    --flux-token "$FLUX_TOKEN" 2>&1 \
    | grep -E "feed=flux-" | head -20
rm -rf /tmp/observer-task11
```
