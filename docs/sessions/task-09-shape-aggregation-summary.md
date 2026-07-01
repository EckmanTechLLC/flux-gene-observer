# Task 09 Summary — Shape Registry: Aggregation Shape

**Status**: implemented, awaiting deploy
**Date**: 2026-05-09
**`cargo check`**: 11 gene-core warnings (baseline), 0 observer-gene warnings

## What Changed

Moved aviation (9 signals, IDs 70–78) and ships (6 signals, IDs 90–95) onto the
catalog model via a new `aggregation` shape. The `signal/aviation.rs` and
`signal/ships.rs` pollers are deleted; `signal/mod.rs` shrinks to a single module
declaration (`flux_multi`).

## Files Created

| File | Notes |
|------|-------|
| `observer-gene/src/shape/aggregation.rs` | `AggregationPoller` + `AggregateSpec` |

## Files Edited

| File | Change |
|------|--------|
| `observer-gene/src/shape/mod.rs` | Added `pub mod aggregation;`, `NormalizeRef` enum, `AggregateConfig` struct, `aggregate` field on `FeedConfig`, `aggregation` dispatch arm in `build_pollers`, `flux-aviation`/`flux-ships` decay cases in `decay_for_namespace` |
| `observer-gene/feeds.toml` | 3 named normalize recipes (`aviation_speed`, `aviation_altitude`, `ships_speed`) + 6 aggregation feeds (3 aviation + 3 ships) |
| `observer-gene/src/signal/mod.rs` | Removed `pub mod aviation; pub mod ships;` |
| `observer-gene/src/main.rs` | Removed 15 `SIG_AIR_*`/`SIG_SHIP_*` consts; removed aviation/ships bus registrations from `build_bus()`; removed `AviationPoller`/`ShipsPoller` imports + instantiation; removed `.poll()` calls from tick loop; updated `verify_registry_consts` comment (aviation/ships now catalog-verified); replaced `SIG_AIR_COUNT_EUROPE`/`SIG_SHIP_COUNT_NORTH_SEA` in action space with inline `SignalId(70)`/`SignalId(90)` |

## Files Deleted

- `observer-gene/src/signal/aviation.rs`
- `observer-gene/src/signal/ships.rs`

## Key Design Decisions

### Registry Key Format
Aggregation signals use `{namespace}#agg.{name}` (no entity component).
`registry.register(namespace, "", "agg.count")` → `make_key` returns
`"flux-aviation-europe#agg.count"` → pre-seeded ID 70. The empty-entity
branch already existed in `make_key` from task-06.

### `NormalizeRef` — Inline + Named Mix
Per-zone count divisors are unique, so the task spec uses inline recipes:
```toml
normalize = { type = "linear_range", min = 0.0, max = 10000.0 }
```
Speed/altitude divisors are shared, using named recipes:
```toml
normalize = "aviation_speed"
```
Implemented as `#[serde(untagged)] enum NormalizeRef { Named(String), Inline(NormalizeRecipe) }`.
`serde` tries `String` first (succeeds for names), then `NormalizeRecipe` (succeeds for tables).

### Decay Rates
Added to `decay_for_namespace`: `flux-aviation` → 0.002, `flux-ships` → 0.001.
Matches the values previously hardcoded in `build_bus()`.

### Startup Log
Each of the 6 feeds emits:
```
INFO feed=flux-aviation-europe shape=aggregation entities=* signals=3
```
Note `entities=*` — aggregation feeds don't enumerate specific entities;
count is dynamic at poll time.

## State After This Task

`observer-gene/src/signal/` now contains only `flux_multi.rs` and `mod.rs`.
All Flux-owned namespace feeds are catalog-driven. `feeds.toml` covers:
- weather (18) + economic (5) + internet (1) = 24 (task-07)
- crypto (42) + stocks (10) + commodities (6) = 58 (task-08)
- aviation (9) + ships (6) = 15 (task-09)
- **Total: 97 catalog signals** (plus 4 earthquakes + 5 internal = 106 seeded)

## Deploy Procedure

Standard (same as tasks 03/05/07/08):
1. Build on ubuntu-dev (.13): `~/.cargo/bin/cargo build --release`
2. rsync to etl@192.168.50.107:~/observer-gene/observer-gene.new
3. On .107: stop, swap binary, start, check logs

Post-deploy verification:
```bash
# 12 feed log lines (6 from task-07/08 + 6 aviation/ships)
sudo journalctl -u observer-gene --since "2 minutes ago" | grep "feed=flux-"

# Aviation/ships signal IDs present in bus
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fstate \
  | jq '.properties.signal_drivers' \
  | grep -E '"7[0-8]"|"9[0-5]"'

# Deleted pollers are gone
strings target/release/observer-gene | grep -E "AviationPoller|ShipsPoller"
# Expected: empty
```
