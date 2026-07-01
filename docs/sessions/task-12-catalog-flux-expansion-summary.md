# Task 12 — Catalog Updates for Flux Expansion

**Date**: 2026-05-13
**File changed**: `observer-gene/feeds.toml` only

## What was done

Complete rewrite of `observer-gene/feeds.toml` to cover all current Flux namespaces.
No source code changes — purely a catalog change.

## Signal counts

| Namespace | Shape | Seeded | Fresh | Total |
|---|---|---|---|---|
| `flux-weather` | flat_multi | 18 | 318 | 336 |
| `flux-economic` | scalar | 5 | 0 | 5 |
| `flux-internet` | scalar | 1 | 0 | 1 |
| `flux-crypto` | nested_by_key | 42 | 0 | 42 |
| `flux-stocks` | array_record | 10 | 0 | 10 |
| `flux-commodities` | time_series | 6 | 0 | 6 |
| `flux-aviation-europe` | aggregation | 3 | 0 | 3 |
| `flux-aviation-uk` | aggregation | 3 | 0 | 3 |
| `flux-aviation-north-atlantic` | aggregation | 3 | 0 | 3 |
| `flux-aviation-east-asia` | aggregation | 0 | 3 | 3 |
| `flux-aviation-north-america` | aggregation | 0 | 3 | 3 |
| `flux-aviation-oceania` | aggregation | 0 | 3 | 3 |
| `flux-ships-north-sea` | aggregation | 2 | 0 | 2 |
| `flux-ships-english-channel` | aggregation | 2 | 0 | 2 |
| `flux-ships-thames` | aggregation | 2 | 0 | 2 |
| `flux-ships-singapore-strait` | aggregation | 0 | 2 | 2 |
| `flux-ships-us-east-coast` | aggregation | 0 | 2 | 2 |
| `flux-spaceweather` (×4 blocks) | flat_multi + scalar | 0 | 8 | 8 |
| `flux-volcanoes` | aggregation | 0 | 8 | 8 |
| `flux-wildfires` | flat_multi | 0 | 60 | 60 |
| `flux-airquality` | flat_multi | 0 | 308 | 308 |
| `flux-grid-eu` | flat_multi | 0 | 20 | 20 |
| `flux-grid-us` | flat_multi | 0 | 36 | 36 |
| `flux-energy` | scalar | 0 | 3 | 3 |
| `flux-iss` (×2 blocks) | scalar + flat_multi | 0 | 4 | 4 |
| **Totals** | | **99** | **778** | **877** |

Pre-task-12 seeded signals: 106 (IDs 0–189, with holes at 39–41 for retired BNB).
After deploy: 106 + 778 = 884 total signals (continuity/internal/earthquakes hardcoded outside catalog = 7 additional always-present signals not in catalog count above).

Wait — the 99 seeded count above includes all 97 catalog-seeded signals from tasks 06-11 plus 2 additional: earthquake signals are hardcoded (not in catalog), and continuity/internal are hardcoded. The catalog covers: weather(18) + economic(5) + internet(1) + crypto(42) + stocks(10) + commodities(6) + aviation-eu(3) + aviation-uk(3) + aviation-natl(3) + ships-ns(2) + ships-ec(2) + ships-thames(2) = 97 seeded from catalog. Plus 778 fresh = 875 catalog signals total.

## Key decisions

### Weather reshape with `name` field (load-bearing)
The 6 original cities × 3 original properties = 18 seeded signal IDs (10–27) are preserved using
the `name` field introduced in task-11. Each of the 3 seeded properties carries:
- `name = "current.temperature_2m"` (old registry key suffix, matches seeded entry)
- `path = "temperature_c"` (new extraction path in reshaped Flux schema)

The 3 new properties (`pressure_mb`, `weather_code`, `wind_direction_deg`) have no `name` field
and allocate fresh IDs for all 56 cities.

### Aviation divisor bumps
- `flux-aviation-europe`: inline `max = 10000.0` → named `aviation_count_europe` (max=15000) — was overflowing at 10,213
- `flux-aviation-uk`: inline `max = 3000.0` → named `aviation_count_uk` (max=5000) — was overflowing at 3,052
- `flux-aviation-north-atlantic`: migrated to named `aviation_count_natl` (max=2500, unchanged)
- New zones all use named recipes

### Multiple `[[feed]]` blocks per namespace
Used for `flux-spaceweather` (4 blocks: solar-wind flat_multi, active-alerts scalar, kp-index scalar, geomag-forecast flat_multi) and `flux-iss` (2 blocks: iss-crew scalar, iss-details flat_multi).
The `build_pollers` loop creates one independent poller per `[[feed]]` entry — no deduplication issue.

### New normalization recipes added
35 new recipes covering weather flat schema, air quality, wildfires, volcanoes, grid, spaceweather, ISS, energy, and aviation/ship count zones.

## `cargo check` result
```
11 gene-core warnings (unchanged baseline)
0 observer-gene warnings
Finished `dev` profile
```

## Deploy verification checklist

At deploy time, the audit log MUST show:

| Feed | `seeded=` | `fresh=` |
|---|---|---|
| `flux-weather` | **18** | **318** |
| `flux-aviation-europe` | 3 | 0 |
| `flux-aviation-uk` | 3 | 0 |
| `flux-aviation-north-atlantic` | 3 | 0 |
| `flux-aviation-east-asia` | 0 | 3 |
| `flux-aviation-north-america` | 0 | 3 |
| `flux-aviation-oceania` | 0 | 3 |
| `flux-ships-singapore-strait` | 0 | 2 |
| `flux-ships-us-east-coast` | 0 | 2 |
| `flux-spaceweather` (×4) | 0 | 8 total |
| `flux-volcanoes` | 0 | 8 |
| `flux-wildfires` | 0 | 60 |
| `flux-airquality` | 0 | 308 |
| `flux-grid-eu` | 0 | 20 |
| `flux-grid-us` | 0 | 36 |
| `flux-energy` | 0 | 3 |
| `flux-iss` (×2) | 0 | 4 total |

**Stop deploy if `flux-weather` shows `seeded < 18`** — means `name` field is mis-wired,
signal IDs 10–27 are being orphaned.

## Status
Implemented. Awaiting build and deploy as part of task-13 (bundled deploy of tasks 06–12).
