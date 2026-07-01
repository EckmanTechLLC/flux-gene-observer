# Task 07 — Shape Registry: scalar/flat_multi + Catalog + Filter

**Status**: implemented, `cargo check` clean  
**Date**: 2026-05-09  
**Depends on**: task-06 (signal registry) ✅

## What changed

### New files

| File | Purpose |
|---|---|
| `observer-gene/feeds.toml` | Catalog — filter, normalize recipes, 3 feeds |
| `observer-gene/src/shape/mod.rs` | `ShapePoller` trait, catalog types, `EntityFilter`, `parse_catalog`, `build_pollers` |
| `observer-gene/src/shape/normalize.rs` | `NormalizeRecipe::LinearRange` |
| `observer-gene/src/shape/path.rs` | Dotted JSON path resolver (`resolve`, `resolve_f64`, `split_first`) |
| `observer-gene/src/shape/flat_multi.rs` | `FlatMultiPoller` — 6 entities × 3 props (weather) |
| `observer-gene/src/shape/scalar.rs` | `ScalarPoller` — N entities × 1 signal (economic, internet) |

### Deleted files

- `observer-gene/src/signal/weather.rs` — 18 signals now via catalog
- `observer-gene/src/signal/economic.rs` — 5 signals now via catalog
- `observer-gene/src/signal/internet.rs` — 1 signal now via catalog

### Edited files

**`observer-gene/Cargo.toml`**: added `toml = { workspace = true }` (already in workspace deps)

**`observer-gene/src/signal/flux_multi.rs`**:
- Added `use crate::shape::EntityFilter;`
- `MultiFluxState` gained `filter: EntityFilter` field
- Added `MultiFluxState::with_filter(filter)` constructor
- `new()` now calls `with_filter` with legacy passthrough filter (keeps `Default` working)
- Removed `is_handled()` function
- `handle_message`: filter check and insert now under a single lock acquisition via `s.filter.accepts(entity_id)`

**`observer-gene/src/signal/mod.rs`**: removed `pub mod weather; pub mod economic; pub mod internet;`

**`observer-gene/src/main.rs`**:
- Added `mod shape;`
- Added `const FEEDS_CATALOG_TOML: &str = include_str!("../feeds.toml");`
- Removed `use crate::signal::{weather,economic,internet}::*` imports
- Added `use crate::shape::EntityFilter;`
- Removed 18 `SIG_WEATHER_*`, 5 `SIG_ECON_*`, 1 `SIG_INTERNET_*` consts (now registry-driven)
- `verify_registry_consts`: removed weather/economic/internet checks (verified by `build_pollers` panic instead)
- `build_bus()`: removed weather/economic/internet registration blocks
- `main()`: `let registry` → `let mut registry` (needed for `build_pollers`)
- `main()`: added `let catalog = shape::parse_catalog(FEEDS_CATALOG_TOML)?;`
- `main()`: `MultiFluxState::new()` → `MultiFluxState::with_filter(EntityFilter::from_config(&catalog.filter))`
- `main()`: removed `weather_poller`, `economic_poller`, `internet_poller` construction
- `main()`: added `let mut shape_pollers = shape::build_pollers(&catalog, &mut registry, multi_state.clone(), &mut bus)?;`
- Tick loop: replaced 3 poller calls with `for p in shape_pollers.iter_mut() { p.poll(&mut bus); }`

## Design details

### Registry key format (matches seed exactly)
- `flat_multi`: `{namespace}/{entity}#{property.path}` → `flux-weather/new-york#current.temperature_2m` → ID 10
- `scalar`: `{namespace}/{entity}#{path}` → `flux-economic/us-consumer-sentiment#value` → ID 140, `flux-internet/global-traffic#result.summary_0.mobile` → ID 150

### Path resolution
- First dotted segment = top-level property key in `state.entities[entity_id]` HashMap
- Remaining segments = nested JSON path within that property's `Value`
- `parse_string = true` → `v.as_str()?.parse::<f64>().ok()` (used for internet mobile %)

### Normalization
- All three feeds use `NormalizeRecipe::LinearRange { min, max }` → `(val - min) / (max - min).clamp(0,1)`
- Weather: `temp_celsius` (-40→40), `wind_kmh` (0→100), `percent_0_100` (0→100)
- Economic: per-indicator ranges matching existing `EconomicPoller` constants
- Internet: `percent_0_100` (string-parsed)

### EntityFilter (replaces `is_handled()`)
- Catalog `[filter]`: `include_prefixes=["flux-"]`, `exclude_prefixes=["flux-devices/","flux-core/"]`, `include_exact=["knowledge-gene/steer"]`
- Behaviorally equivalent to old `is_handled()` for all entities that exist today
- Also accepts `flux-earthquakes/` (harmless — no shape poller reads from it)

### Startup log (per feed)
```
INFO feed=flux-weather shape=flat_multi entities=6 signals=18
INFO feed=flux-economic shape=scalar entities=5 signals=5
INFO feed=flux-internet shape=scalar entities=1 signals=1
```

### ID stability guard
`build_pollers` checks `registry.get_by_key(full_key)` before calling `register()`. If the key is absent (would allocate a fresh ID), it panics at startup. For task-07, all 24 signals are pre-seeded; any panic = key format mismatch bug.

## cargo check result
- 11 gene-core warnings (unchanged baseline)
- **0 observer-gene warnings**

## Deploy instructions

Build on ubuntu-dev (.13):
```bash
cd /home/etl/projects/gene-observer
~/.cargo/bin/cargo build --release
```

Deploy to etl-flux (.107):
```bash
rsync -avz target/release/observer-gene etl@192.168.50.107:~/observer-gene/observer-gene.new
ssh etl@192.168.50.107 '
  sudo systemctl stop observer-gene
  mv ~/observer-gene/observer-gene ~/observer-gene/observer-gene.bak
  mv ~/observer-gene/observer-gene.new ~/observer-gene/observer-gene
  chmod +x ~/observer-gene/observer-gene
  sudo systemctl start observer-gene
  sudo journalctl -u observer-gene -n 30
'
```

Post-deploy verification:
```bash
# Feed log lines
sudo journalctl -u observer-gene --since "2 minutes ago" | grep -E "feed=flux-(weather|economic|internet)"
# Expected: 3 lines

# Service active
sudo systemctl status observer-gene

# State still publishing
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fstate | jq '.properties | {tick, dominant}'
```

## Rollback
```bash
sudo systemctl stop observer-gene
mv ~/observer-gene/observer-gene.bak ~/observer-gene/observer-gene
sudo systemctl start observer-gene
```
