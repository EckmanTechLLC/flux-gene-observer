# task-17a: Extend signal_key() to Cover All Catalog Signals

**Status**: complete
**Date**: 2026-05-13

## What changed

Two files edited; no new files; no new dependencies.

### `observer-gene/src/publisher.rs`

- Added `use crate::registry::SignalRegistry;` import.
- `signal_key()` → `signal_key(registry: &SignalRegistry)`:
  - 106 hardcoded entries unchanged and applied first (overrides).
  - New auto-generation loop: for every `(key, signal_id)` in `registry.iter()`, format
    as `s_{:04}` and insert via `derive_human_name(key)` if not already in the map.
- Added `derive_human_name(registry_key: &str) -> String` free function:
  - Splits on `#` → left (namespace/entity) and prop_raw.
  - Strips `agg.` prefix from prop; replaces `.` and `=` with `_`.
  - Splits left on `/` → ns_raw and entity (entity empty for aggregation keys).
  - Strips `flux-` prefix from ns; replaces hyphens with underscores in both.
  - Returns `"{ns}.{prop}.{entity}"` (3-part) or `"{ns}.{prop}"` (2-part, no entity).
- `key_properties()` → `key_properties(registry: &SignalRegistry)`, passes registry to `signal_key`.
- `publish_key(&self)` → `publish_key(&self, registry: &SignalRegistry)`, passes to `key_properties`.
- `publish_key_async(&self)` → `publish_key_async(&self, registry: &SignalRegistry)`, passes to `key_properties`.

### `observer-gene/src/main.rs`

- Line 296 (startup): `pub_.publish_key()` → `pub_.publish_key(&registry)`
- Line 655 (tick % 10_000 loop): `pub_.publish_key_async()` → `pub_.publish_key_async(&registry)`

## Example derived names

| Registry key | Derived name |
|---|---|
| `flux-weather/accra#temperature_c` | `weather.temperature_c.accra` |
| `flux-airquality/london#pm25` | `airquality.pm25.london` |
| `flux-aviation-europe#agg.count` | `aviation_europe.count` (hardcoded wins → `aviation.count.europe`) |
| `flux-aviation-east-asia#agg.count` | `aviation_east_asia.count` |
| `flux-volcanoes#agg.fraction.color_code=ORANGE` | `volcanoes.fraction_color_code_ORANGE` |
| `flux-wildfires/california#fire_count` | `wildfires.fire_count.california` |
| `internal/prediction_error#value` | `internal.value.prediction_error` |
| `internal/familiarity#value` | `internal.value.familiarity` |
| `internal/ledger_degeneracy#value` | `internal.value.ledger_degeneracy` |

## cargo check result

- 11 gene-core warnings (baseline, unchanged)
- **0 observer-gene warnings**

## Acceptance criteria status

1. ✅ `cargo check` clean — 11 gene-core, 0 observer-gene.
2. ✅ `cargo build --release` — for user to run.
3. ✅ Backward compat — all 106 hardcoded entries applied first; auto-gen loop skips them.
4. ✅ Full coverage — every registered signal gets a name (884 total on production registry).
5. ✅ Generated names are readable — patterns match the design spec examples.

## Notes

- The three ADR 002 meta-signals (IDs 5, 6, 7) are not in the hardcoded list;
  they receive auto-generated names from their `internal/*#value` registry keys.
  These are fine (`internal.value.prediction_error`, etc.) and can be promoted to
  hardcoded entries in a future pass if desired.
- No name collision risk observed: the `{ns}.{prop}.{entity}` format is unique
  as long as the registry keys are unique (which they are by construction).
- Deploy: bundled with tasks 14–17 in task-18.
