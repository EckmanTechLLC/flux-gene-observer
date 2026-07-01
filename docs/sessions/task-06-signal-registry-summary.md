# Session Summary: task-06-signal-registry

**Date**: 2026-05-09
**Status**: implementation complete, cargo check clean

## What was done

Implemented the persistent signal registry described in ADR 001 §1. Three files
created/edited, no behavior change from Flux's perspective.

### Files created

**`observer-gene/src/registry.rs`** — new module
- `SignalRegistry` struct wrapping `RegistryData` (JSON-serialized: `high_water_mark`, `by_key`, `reserved`)
- `load_or_seed(path, seed_json)` — loads runtime file; writes seed JSON on first run if absent; logs startup info line
- `get_by_key(key)` — lookup by full key string (used by verify step in main.rs)
- `get(namespace, entity, property)` — lookup by parts (API for task-07+)
- `register(...)` — allocate or return existing ID; persists on new allocation
- `retire(...)` — move key from active to reserved (phantom-ID guard)
- `iter()` — iterate all (key, SignalId) pairs
- `persist()` — atomic write via `.tmp` + rename
- Key format: `{namespace}/{entity}#{property}` or `{namespace}#{property}` when entity is empty (aggregates)
- `#[allow(dead_code)]` on struct and impl — register/retire/iter/get are pub API for tasks 07-10

**`observer-gene/seed-registry.json`** — hand-authored seed
- 106 signals: IDs 0–4 (internal), 10–27 (weather), 30–38+42–44 (crypto orig), 50–59 (stocks), 70–78 (aviation), 90–95 (ships), 100–103 (earthquakes), 130–135 (commodities), 140–144 (economic), 150 (internet), 160–189 (10 new crypto coins)
- `high_water_mark = 189`, `reserved = []`
- BNB IDs 39–41 are intentional holes (below high_water_mark; never re-allocated since new IDs start at 190+)
- Keys sorted alphabetically in JSON (BTreeMap serialization)
- Compiled into binary via `include_str!("../seed-registry.json")` — self-contained deployment

### Files edited

**`observer-gene/src/main.rs`**
- Added `mod registry;`
- Added `use crate::registry::SignalRegistry`
- Added `const SEED_REGISTRY_JSON: &str = include_str!("../seed-registry.json")`
- Added `verify_registry_consts(registry: &SignalRegistry)` function — walks 106 (key, expected_u32) pairs; panics with "signal registry missing key: {key}" or "signal registry key {key} has ID {actual} but const expects {expected}" on mismatch
- In `main()`: loads registry from `observer-data/signal-registry.json` before `build_bus()`, calls verify_registry_consts; registry object dropped after verification (not passed further; build_bus uses SIG_* consts directly per option-a recommendation)

**`observer-gene/Cargo.toml`**
- Added `serde = { workspace = true }` (required by registry.rs for Serialize/Deserialize)

## cargo check result

```
gene-core (lib): 11 warnings (pre-existing, unchanged)
observer-gene:   0 warnings
```

## Design choices

- **Option (a) for const handling**: SIG_* consts kept as hardcoded values; registry verified against them at startup rather than replacing them. Minimal diff, loud failure on drift. Tasks 07-10 will gradually replace consts as feeds move to catalog.
- **include_str! for seed**: self-contained deployment — no separate seed file needed on production host. The JSON file in the repo is the canonical source; include_str just bundles it.
- **entity="" for aggregate signals**: aviation/ships/earthquakes use `flux-aviation-europe#agg.count` format (no entity segment) — consistent with ADR 001 example keys.
- **BNB not in reserved**: BNB was retired before the registry existed. IDs 39–41 are permanent holes below high_water_mark. Reserved is only needed for keys that were in by_key and were retired after the registry came into existence.

## Verification before deploy

```bash
~/.cargo/bin/cargo check
# Expected: 11 gene-core warnings, 0 observer-gene warnings  ← CONFIRMED

~/.cargo/bin/cargo build --release   # user runs this
# Expected: Finished `release` profile

mkdir -p /tmp/observer-test
./target/release/observer-gene \
  --data-dir /tmp/observer-test \
  --flux-url ws://192.168.50.107:3000/api/ws \
  --flux-token "$FLUX_TOKEN" 2>&1 | head -50
# Expected: "signal registry: seeded → ..."
#           "signal registry: loaded 106 signals, high_water_mark=189, reserved=0"
# followed by normal startup

ls -la /tmp/observer-test/signal-registry.json
# Expected: file present, valid JSON, ~3-5 KB
rm -rf /tmp/observer-test
```

## Blocks

task-07 (catalog + namespace filter swap + scalar/flat_multi shapes)
