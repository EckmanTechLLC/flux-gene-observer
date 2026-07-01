# ADR 001 — Dynamic Signal Registry & Shape-Driven Pollers

**Status**: accepted (2026-05-09)
**Author**: foundation session, in collaboration with @matt
**Related**: CLAUDE.md signal-ID table, OBSERVER_GENE.md "Core Principle: Empirical Feeds Only"

---

## Context

Observer-gene currently registers ~80 signals via `const SIG_*: SignalId = SignalId(N);`
declarations in `main.rs`, hand-written per-feed pollers in `signal/{weather,
crypto, stocks, aviation, ships, commodities, economic, internet}.rs`, and a
hardcoded prefix allowlist in `flux_multi.rs::is_handled()`. Adding a single
new Flux feed requires touching 4–5 files, allocating ID blocks by hand,
writing a custom poller, and a full rebuild + redeploy.

Flux is now expanding rapidly — many new namespaces incoming, of varied
shapes. The hand-coding approach won't scale. At the same time:

1. **Some namespaces must be excluded** (`pure-ash`, `pure-jade`,
   `flux-devices`, `flux-core`, plus the gene-internal `observer-gene/` and
   `knowledge-gene/` outside of the steer entity). Filter must be subtractive
   relative to "all `flux-*`", not just additive.
2. **Some feeds genuinely need code** — the Kraken nested-by-pair format,
   stocks `results[0].c`, time-series arrays, and aggregation across many
   entities cannot be expressed by a single declarative path.
3. **Flux is read-only world state.** Observer cannot ask Flux to publish a
   schema or any other metadata; it must infer everything from the data it
   sees.
4. **The symbol ledger must keep working.** Symbols (`Φ_NNNN`) reference
   signals by `SignalId`. Existing IDs 0–4 and 10–189 cannot shift, and
   removed feeds cannot have their IDs reused, or pattern_ids in the ledger
   become phantoms.

The constraint that drives everything else is (4): signal IDs are forever.
Anything dynamic must allocate above the existing high-water mark and
persist that allocation across restarts.

---

## Decision

Restructure observer-gene's signal layer around three new components, all
observer-side, no gene-core changes (except point 4 in the action-space
design below, which is a thin SystemOp variant).

### 1. Persistent signal registry

A single JSON file at `observer-data/signal-registry.json`:

```json
{
  "high_water_mark": 189,
  "by_key": {
    "flux-weather/new-york#current.temperature_2m": 10,
    "flux-weather/new-york#current.wind_speed_10m": 11,
    "...": "..."
  },
  "reserved": []
}
```

- **Key format**: `{namespace}/{entity}#{property_path}`, where
  `property_path` is the dotted path from `properties` (e.g.
  `current.temperature_2m`) or a synthetic suffix for aggregates (e.g.
  `#agg.count`, `#agg.mean.speed_ms`, `#derived.zscore`).
- **Allocation**: `register(namespace, entity, property)` returns the
  existing `SignalId` if present, otherwise allocates `high_water_mark + 1`,
  inserts, persists, returns. Atomic on disk (write-temp-then-rename).
- **Reservation**: when a feed is retired, its key moves from `by_key` into
  `reserved` so the ID is never re-allocated. No active code path uses the
  registry-by-ID lookup, so leaving the slot reserved is purely a phantom-ID
  guard.
- **Seeding**: the seed registry is hand-authored (Option A) by walking
  CLAUDE.md's signal-ID table once. The seed becomes
  `observer-gene/seed-registry.json` in the repo, copied to
  `observer-data/signal-registry.json` on first run if the latter doesn't
  exist. After that the runtime file is the source of truth.

### 2. Namespace filter — positive prefix + explicit excludes + allowlist

Replace `flux_multi.rs::is_handled()` with a configurable filter. Default
config (committed to repo):

```toml
[filter]
include_prefixes = ["flux-"]
exclude_prefixes = ["flux-devices/", "flux-core/"]
include_exact    = ["knowledge-gene/steer"]
exclude_exact    = []
```

Logic: subscribe to entity_id if
`(matches any include_prefix OR matches any include_exact) AND not (matches any exclude_prefix OR matches any exclude_exact)`.

This handles:
- All current and future `flux-*` Flux-owned namespaces (positive prefix)
- Per-namespace exclusions (`flux-devices`, `flux-core`)
- `pure-ash`, `pure-jade`, `observer-gene/`, `knowledge-gene/state` etc.
  excluded automatically because they don't match `flux-*`
- The single non-Flux entity we intentionally observe
  (`knowledge-gene/steer`)

### 3. Shape registry + catalog

Six built-in shapes, each implemented as a poller class that takes a config
struct. Adding a new feed of a known shape = catalog entry only. Adding a
new shape = small enum addition + handler.

| Shape | Description | Replaces |
|---|---|---|
| `scalar` | One numeric value at a fixed JSON path per entity | `economic.rs`, `internet.rs` |
| `flat_multi` | Multiple numeric values at fixed paths per entity | `weather.rs` |
| `nested_by_key` | Value lives under an entity-specific key (Kraken pair, CoinGecko ID) | `crypto.rs` |
| `array_record` | Value at `path[0].field` | `stocks.rs` |
| `time_series` | Value at `data[0].value`, may need string→f64 parse | `commodities.rs` |
| `aggregation` | Reduce many entities → count/mean/fraction; emit per-zone aggregates | `aviation.rs`, `ships.rs` |

Each shape has a fixed set of normalization options chosen by name from a
small library (`temp_celsius`, `wind_kmh`, `percent`, `zscore_window_20`,
`fixed_range`, `ema_relative`, `divisor`, `aggregation_count`,
`aggregation_speed`). New normalizations are added as named functions in the
library; never inline math in the catalog.

#### Catalog file: `observer-gene/feeds.toml` (committed)

```toml
[[feed]]
namespace = "flux-weather"
shape = "flat_multi"
entities = ["new-york", "london", "tokyo", "sydney", "dubai", "los-angeles"]
properties = [
  { path = "current.temperature_2m",     normalize = "temp_celsius" },
  { path = "current.wind_speed_10m",     normalize = "wind_kmh" },
  { path = "current.relative_humidity_2m", normalize = "percent" },
]

[[feed]]
namespace = "flux-economic"
shape = "scalar"
property = "value"
entities = [
  { name = "us-consumer-sentiment", normalize = { type = "fixed_range", min = 0, max = 110 } },
  { name = "us-initial-claims",     normalize = { type = "fixed_range", min = 0, max = 500_000 } },
  # ...
]

[[feed]]
namespace = "flux-aviation-europe"
shape = "aggregation"
aggregates = [
  { name = "count",        normalize = { type = "divisor", value = 10_000 } },
  { name = "mean.speed_ms",    normalize = { type = "divisor", value = 300 } },
  { name = "mean.altitude_m",  normalize = { type = "divisor", value = 12_500 } },
]
```

Earthquakes (`flux.rs`) and the continuity/meta/drive signals (IDs 0–4) stay
hardcoded — they're not "feeds" in the sense above.

### 4. Action-space — parameterized perception meta-actions

Today `AdjustDecay`/`AdjustBaseline` target specific signals via separate
`Action` instances per target (~20 hand-picked). With possibly hundreds of
new signals this would explode the action space.

Decision: introduce **parameterized perception meta-actions** modeled on
`CoinDerivedSignal`'s runtime-allocation pattern:

- One `SystemOp::AdjustDecay { signal_id, delta }` variant accepts a target
  ID at execution time
- `AdjustBaseline` similarly parameterized
- `build_action_space()` materializes a curated subset of signals as
  individual actions (same hand-picked targets as today's catalog) plus a
  small pool of "open-slot" actions allocated dynamically against
  symbol-driven targets — the regulation engine learns which signals are
  worth adjusting

This requires a single small variant change in
`gene-core/src/regulation/action.rs`. It's the **fourth gene-core patch**;
acceptable because it's parameter-shape only (no logic change) and the
patch list is now tracked in `.odin/memory.md` and propagated only when
explicit.

The "open-slot" allocation lives entirely in observer-gene.

### 5. Migration approach

Existing IDs 10–189 frozen; new feeds allocate above 189. The seed registry
is hand-authored from CLAUDE.md to perfect fidelity (Option A from the
discussion).

The migration is implemented in 5 deploys:

| Task | Scope | Behavior at end |
|---|---|---|
| **task-06** | Persistent registry module + seed file + load/save + adopted by `build_bus()` (no shape changes yet) | Bus registers exactly the same signals as today, but via the registry. Existing pollers untouched. Cuts no metal in dynamism but proves the foundation. |
| **task-07** | Catalog file + namespace filter swap + `scalar` and `flat_multi` shape pollers | `flux-weather`, `flux-economic`, `flux-internet` move to the new model. Old `weather.rs`/`economic.rs`/`internet.rs` removed. |
| **task-08** | `nested_by_key`, `array_record`, `time_series` shape pollers | `flux-crypto`, `flux-stocks`, `flux-commodities` move over. Old custom pollers removed. |
| **task-09** | `aggregation` shape | `flux-aviation-*`, `flux-ships-*` move over. Old custom pollers removed. |
| **task-10** | Parameterized `AdjustDecay`/`AdjustBaseline` + curated action space + open-slot allocator | Action space scales with signal count without explosion. |

After task-09, every Flux-owned namespace is governed by the catalog; new
feeds of known shape need only a catalog entry. After task-10, perception
meta-actions scale with the signal set.

Each task is a discrete deploy with rollback to the prior `.bak` binary.
Symbol ledger stability verified at each deploy by checking that the live
expression engine continues to emit the same dominant symbol IDs across the
upgrade boundary.

---

## Consequences

### Wins

- New feed of a known shape: catalog entry only, no code, no rebuild of
  observer logic (still rebuild + redeploy, but the source change is
  declarative).
- Symbol ledger preserved — every existing `Φ_NNNN`'s pattern_id keeps
  resolving to the same signal IDs.
- Negative filter handles excludes cleanly; positive `flux-*` prefix
  expresses the actual rule.
- Action space scales with signal count instead of exploding.
- `flux_multi.rs` becomes a thin transport; all shape logic lives in the
  shape registry.

### Costs

- **Fourth gene-core patch** (parameterized SystemOp variants). Tracked in
  memory; do-not-propagate.
- **Persistent file** (`observer-data/signal-registry.json`) added to
  runtime state — must survive restarts and never be lost. Backup discipline
  becomes part of operations.
- **Catalog drift risk**: catalog entries can fall out of sync with reality
  (a renamed entity, a property that disappeared). Mitigated by a startup
  sanity log: "feed configured for entities X but Flux has Y" — info-level,
  no behavior change.
- **Migration window**: between task-06 and task-09, the codebase has both
  old custom pollers and new catalog-driven ones. Each task removes its
  predecessors' code so there's no permanent dual-mode.

### Non-goals

- **No Flux-side changes.** Observer infers everything from raw data.
- **No auto-discovery without catalog entry.** A new namespace appearing in
  Flux that isn't in the catalog is logged once, then ignored. Curation
  stays explicit.
- **No runtime catalog reload.** Catalog is read once at startup; changes
  require redeploy. (Future enhancement if needed.)
- **No removal of the earthquake feed's separate WS path.** It stays
  hardcoded — it's not a "Flux entity feed" in the same sense.

---

## Alternatives considered and rejected

- **(B) Auto-allocate IDs on first WS sight.** Order-dependent, unstable
  across restarts before the registry persists, fragile.
- **(D) Flux self-describes via per-namespace schema entities.** Cleanest
  long-term but requires Flux-side cooperation. Ruled out by user
  preference: Flux stays read-only world state.
- **Hash entity_id → ID space.** Collision risk; opaque IDs make debugging
  harder; fundamentally incompatible with the symbol-ledger persistence.
- **One action per signal.** Action space explodes; selector becomes
  unwieldy.

---

## Open issues for follow-up

- **Catalog format evolution.** TOML may need extension for normalizations
  with more parameters. Cross that bridge as it appears.
- **Observability.** Each new shape poller should emit a startup summary
  ("feed=flux-X shape=Y entities=N signals_registered=M") so we can verify
  catalog ↔ live alignment in one log scan.
- **flux-core inclusion.** Excluded by default; revisit if it turns out to
  carry useful aggregates.
- **Steer staleness budget.** Today 100,000 ticks. The first natural KG
  steer consumed 85k. Watch — bump to 200,000 if we see stale rejects in
  the journal.
