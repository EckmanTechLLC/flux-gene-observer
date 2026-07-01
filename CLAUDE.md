# CLAUDE.md — observer-gene

This file is the authoritative development guide. It is not a task or status tracker.

## Development Workflow

For any non-trivial change: **READ → VERIFY → ANALYZE → PROPOSE → IMPLEMENT**

1. **READ** — Read all referenced source files before proposing anything
2. **VERIFY** — Cross-check assumptions against actual code; never rely on memory
3. **ANALYZE** — Understand what the data actually looks like (query Flux; read the signal shapes)
4. **PROPOSE** — State what changes, why, which files are touched, and what the signal/data mappings are
5. **IMPLEMENT** — Minimum change needed; do not refactor or clean up adjacent code

Never implement without explicit approval. Never modify gene-core logic without explicit instruction.

**Build & run rule: only the user builds and runs gene.** Claude may use `cargo check` to verify
compilation but must never execute `cargo build --release` or the binary. Tell the user what to run.
When in doubt about any design decision, re-read OBSERVER_GENE.md first.

---

## What This Is

observer-gene is a pure world-observation binary built on gene-core. No controlled signals.
No trading system. No external actuation. Gene's only goal is self-preservation (continuity IDs 0–2).
All world signals are `weight=0` — they feed the pattern/symbol machinery without driving imbalance.

The output is the symbol ledger: cross-domain co-activation patterns that emerged from observing
diverse empirical feeds simultaneously. Gene publishes its state to Flux for downstream consumers.

See OBSERVER_GENE.md for full design rationale.

---

## Repository Layout

```
gene-observer/                   ← this repo
├── Cargo.toml                   ← workspace root (members: ["gene-core", "observer-gene"])
├── CLAUDE.md                    ← this file
├── OBSERVER_GENE.md             ← planning document (authoritative)
├── observer-gene.toml           ← runtime config (Flux URL, namespace list)
├── gene-core/                   ← copy of gene-core with lib target added
│   ├── Cargo.toml               ← [lib] target = "gene_core"
│   └── src/
│       ├── lib.rs               ← pub mod declarations
│       └── ... (full gene-core source)
├── observer-gene/
│   ├── Cargo.toml               ← path dep: gene-core = { path = "../gene-core" }
│   └── src/
│       ├── main.rs              ← tick loop, signal registration, config loading
│       ├── config.rs            ← Config struct, TOML loading
│       └── signal/
│           ├── mod.rs
│           ├── flux_multi.rs    ← THE new module: multi-namespace Flux subscriber
│           ├── weather.rs       ← WeatherPoller: 6 cities × 3 properties
│           ├── crypto.rs        ← CryptoPoller: 5 assets × 3 properties + EMA/delta
│           ├── stocks.rs        ← StocksPoller: 5 tickers × 2 properties + EMA/delta
│           ├── aviation.rs      ← AviationPoller: 3 zones × 3 aggregates
│           ├── ships.rs         ← ShipsPoller: 3 zones × 2 aggregates
│           ├── earthquakes.rs   ← reuse existing gene-core flux.rs logic
│           ├── commodities.rs   ← CommoditiesPoller: 6 entities × 1 property (z-score)
│           ├── economic.rs      ← EconomicPoller: 5 indicators × 1 scalar
│           └── internet.rs      ← InternetPoller: 1 entity, mobile % extraction
└── observer-data/               ← runtime data (checkpoint, directives, logs, actions.json)
```

gene-core is copied from gene-financial/gene-core (which already has the lib target).
It is referenced via path dependency — not a git submodule.

---

## gene-core Dependency

gene-core in this repo is a copy of `projects/gene-financial/gene-core` which already has:

```toml
# gene-core/Cargo.toml
[lib]
name = "gene_core"
path = "src/lib.rs"
```

And `gene-core/src/lib.rs`:
```rust
pub mod signal;
pub mod regulation;
pub mod pattern;
pub mod symbol;
pub mod selfmodel;
pub mod persistence;
pub mod expression;
```

**Do not modify any gene-core logic** without explicit instruction. Any substantive change
must be discussed, approved, and made first in `projects/gene/gene-core/`, then propagated.

---

## Live Flux Feeds — Actual Data Shapes

**Public instance:** `wss://api.flux-universe.com/api/ws`
**Local dev:** `ws://192.168.50.107:3000/api/ws` (or `ws://localhost:3000/api/ws`)

WebSocket is read-only, no auth required. Subscribe with `{"type":"subscribe","entity_id":"*"}`.
State updates arrive as: `{"type":"state_update","entity_id":"...","property":"...","value":...}`.

### flux-weather/{city} — 6 entities

Cities: `new-york`, `london`, `tokyo`, `sydney`, `dubai`, `los-angeles`

Properties arrive as nested objects. Key property: `current` (JSON object):
```json
{
  "current": {
    "temperature_2m": 7.0,
    "apparent_temperature": 4.9,
    "relative_humidity_2m": 80,
    "wind_speed_10m": 4.8,
    "weather_code": 2
  }
}
```
Extract: `properties["current"]["temperature_2m"]` — the `current` property value is itself a JSON object.
Units: temperature °C, wind km/h, humidity %, weather_code WMO integer.

### flux-crypto/{coin} — 5 entities

Coins: `bitcoin`, `ethereum`, `solana`, `bnb`, `xrp`

Properties arrive nested under the CoinGecko coin ID:
```json
// entity: flux-crypto/bitcoin
{ "bitcoin": { "usd": 68842, "usd_24h_change": 3.84, "usd_24h_vol": 61686348881.7, "usd_market_cap": 1376642941091.3 } }

// entity: flux-crypto/bnb  (note: CoinGecko ID is "binancecoin")
{ "binancecoin": { "usd": 638.38, ... } }

// entity: flux-crypto/xrp  (CoinGecko ID is "ripple")
{ "ripple": { "usd": 1.40, ... } }
```
The property key is the CoinGecko coin ID — **not** the entity path segment. Map explicitly:
`bitcoin→bitcoin`, `ethereum→ethereum`, `solana→solana`, `bnb→binancecoin`, `xrp→ripple`

### flux-stocks/{TICKER} — 5 entities

Tickers: `AAPL`, `TSLA`, `NVDA`, `MSFT`, `SPY`

Properties are Polygon OHLCV wrapped in a results array:
```json
{
  "results": [{ "T": "SPY", "o": 683.09, "h": 686.86, "l": 681.64, "c": 685.99, "v": 83292447.0, "vw": 684.93, "n": 1113880 }],
  "ticker": "SPY"
}
```
Extract: `properties["results"][0]["c"]` for close, `["v"]` for volume, `["vw"]` for VWAP.
These are **prev-day OHLCV** — updated once per day. Normalization uses rolling window.

### flux-aviation-{zone}/{ICAO24-*} — ~12,000 entities across 3 namespaces

Zones: `europe` (7,683), `uk` (2,344), `north-atlantic` (1,975)

Each aircraft entity (flat properties):
```json
{
  "altitude_m": 11277.6,
  "callsign": "EJU52YG",
  "country": "Austria",
  "heading": 174.6,
  "lat": 49.125,
  "lon": 14.22,
  "on_ground": false,
  "speed_ms": 229.43,
  "vertical_rate": 0.33
}
```
Aggregate per namespace: count, mean(speed_ms), mean(altitude_m), fraction(on_ground==false).

### flux-ships-{zone}/{MMSI-*} — ~11,680 entities across 3 namespaces

Zones: `north-sea` (8,824), `english-channel` (1,828), `thames` (1,028)

Each vessel entity (flat properties):
```json
{
  "heading": 257,
  "lat": 50.31,
  "lon": -1.03,
  "name": "NAVIOS CELESTIAL",
  "speed": 7.7,
  "status": "underway-engine",
  "timestamp": "2026-03-02T18:56:44Z"
}
```
Status values observed: `underway-engine`, `anchored`, `moored`, `fishing`, `restricted-maneuverability`
Aggregate per namespace: count, mean(speed), fraction(underway-engine).

### flux-earthquakes/{USGS-ID} — live USGS M2.5+ events

Already implemented in gene-core/src/signal/flux.rs. Reuse as-is.
Properties: `magnitude`, `depth_km`, `sig`, `time` (ms epoch), `lat`, `lon`

### flux-commodities/{commodity} — 6 entities

Entities: `brent-crude`, `wti-crude`, `natural-gas`, `copper`, `corn`, `wheat`

Properties contain a time-series array plus metadata:
```json
{
  "data": [{"date": "2026-02-23", "value": "71.9"}, ...],
  "interval": "daily",
  "name": "Crude Oil Prices Brent",
  "unit": "dollars per barrel"
}
```
Extract: `properties["data"][0]["value"]` (most recent entry) — **string**, parse as f64.
Intervals: brent-crude/wti-crude/natural-gas are daily; copper/corn/wheat are monthly.
Normalization: rolling z-score(20) mapped to [0,1].

### flux-economic/{indicator} — 5 entities

Entities: `us-consumer-sentiment`, `us-initial-claims`, `us-unemployment-rate`, `us-inflation-cpi`, `us-gdp-growth`

Properties are flat FRED scalars:
```json
{ "series_date": "2026-01-01", "series_id": "UMCSENT", "units": "Index 1966=100", "value": 56.4 }
```
Extract: `properties["value"]` — f64 scalar directly. Updated monthly/weekly/quarterly.
Normalization: fixed ranges (see signal table below).

### flux-internet/global-traffic — 1 entity

Cloudflare Radar 24h rolling traffic split:
```json
{
  "result": {
    "summary_0": { "desktop": "59.391148", "mobile": "40.589594", "other": "0.019258" }
  }
}
```
Extract: `properties["result"]["summary_0"]["mobile"]` — string, parse as f64.
Normalization: val / 100 (already a percentage 0–100). Updated hourly.

---

## Signal ID Table

All world signals: `weight=0.0`, `SignalClass::World`. Set at registration. Never change.

### Continuity (IDs 0–2)
Exponential cost. Same as all gene instances.

| ID | Name | Weight |
|----|------|--------|
| 0 | s_continuity | 50.0 |
| 1 | s_integrity | 30.0 |
| 2 | s_coherence | 20.0 |

### Internal (IDs 3–4)

| ID | Name | Weight | Notes |
|----|------|--------|-------|
| 3 | s_meta | 2.0 | MetaSignal prediction confidence |
| 4 | s_drive | 1.0 | RegulationDrive urgency |

### Weather (IDs 10–27, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 10 | s_weather_temp_newyork | (°C + 40) / 80 clamped [0,1] |
| 11 | s_weather_wind_newyork | km/h / 100 clamped [0,1] |
| 12 | s_weather_humidity_newyork | % / 100 |
| 13–15 | london (temp, wind, humidity) | same |
| 16–18 | tokyo (temp, wind, humidity) | same |
| 19–21 | sydney (temp, wind, humidity) | same |
| 22–24 | dubai (temp, wind, humidity) | same |
| 25–27 | los-angeles (temp, wind, humidity) | same |

### Crypto (IDs 30–44, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 30 | s_crypto_price_btc | rolling z-score(20) mapped to [0,1] |
| 31 | s_crypto_vol_btc | vol / EMA(vol,20) / 3 clamped [0,1] |
| 32 | s_crypto_change_btc | (24h_change% + 20) / 40 clamped [0,1] |
| 33–35 | eth (price, vol, change) | same |
| 36–38 | sol (price, vol, change) | same |
| 39–41 | bnb (price, vol, change) | same |
| 42–44 | xrp (price, vol, change) | same |

### Stocks (IDs 50–59, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 50 | s_stock_close_spy | rolling z-score(20) mapped to [0,1] |
| 51 | s_stock_vol_spy | vol / EMA(vol,20) / 3 clamped [0,1] |
| 52–53 | nvda (close, vol) | same |
| 54–55 | aapl (close, vol) | same |
| 56–57 | tsla (close, vol) | same |
| 58–59 | msft (close, vol) | same |

### Aviation (IDs 70–78, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 70 | s_air_count_europe | count / 10000 clamped [0,1] |
| 71 | s_air_speed_avg_europe | mean(speed_ms) / 300 clamped [0,1] |
| 72 | s_air_altitude_avg_europe | mean(altitude_m) / 12500 clamped [0,1] |
| 73–75 | uk (count, speed, altitude) | count / 3000 |
| 76–78 | north-atlantic (count, speed, altitude) | count / 2500 |

### Ships (IDs 90–95, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 90 | s_ship_count_north_sea | count / 10000 clamped [0,1] |
| 91 | s_ship_speed_avg_north_sea | mean(speed knots) / 25 clamped [0,1] |
| 92–93 | english-channel (count, speed) | count / 2500 |
| 94–95 | thames (count, speed) | count / 1500 |

### Earthquakes (IDs 100–103, weight=0)

| ID | Name | Notes |
|----|------|-------|
| 100 | s_quake_rate | reuse FluxPoller from gene-core/signal/flux.rs |
| 101 | s_quake_magnitude | same |
| 102 | s_quake_depth | same |
| 103 | s_quake_sig | same |

### Derived Transforms (IDs 110–129, weight=0)

EMA(9), EMA(21), delta applied to crypto price and stock close signals.
Exact IDs assigned during Phase 4 — document here when allocated.

### Commodities (IDs 130–135, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 130 | s_commodity_price_brent | rolling z-score(20) mapped to [0,1] |
| 131 | s_commodity_price_wti | same |
| 132 | s_commodity_price_natgas | same |
| 133 | s_commodity_price_copper | same |
| 134 | s_commodity_price_corn | same |
| 135 | s_commodity_price_wheat | same |

### Economic (IDs 140–144, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 140 | s_econ_consumer_sentiment | val / 110 clamped [0,1] |
| 141 | s_econ_initial_claims | val / 500000 clamped [0,1] |
| 142 | s_econ_unemployment | val / 15 clamped [0,1] |
| 143 | s_econ_inflation_cpi | (val − 280) / 100 clamped [0,1] |
| 144 | s_econ_gdp_growth | (val + 10) / 20 clamped [0,1] |

### Internet (IDs 150, weight=0)

| ID | Name | Normalization |
|----|------|--------------|
| 150 | s_internet_mobile_pct | val / 100 |

IDs 200+: runtime-allocated by CoinDerivedSignal.

---

## flux_multi.rs — Design

The core new module. Two operating modes per configured feed:

**Aggregation mode** (aviation, ships): Many entities per namespace → scalar aggregates.
- Subscribes to all entities (`entity_id: "*"`)
- Routes `state_update` by namespace prefix
- Per-namespace accumulator: count, running sum/mean of configured properties
- Polled every 10 ticks → writes aggregates to bus

**Extraction mode** (weather, crypto, stocks): Few entities, nested properties → scalar extraction.
- Same WS connection, same subscription
- Routes by exact entity_id prefix
- Per-entity accumulator: stores latest value per property path
- Property path supports one level of nesting: `"current.temperature_2m"` extracts `json["current"]["temperature_2m"]`
- For arrays: `"results[0].c"` — handle explicitly in each poller (not generic)
- Polled every 10 ticks → writes normalized values to bus

**Threading pattern** (same as gene-core/signal/flux.rs — do not deviate):
- `std::thread::spawn` with its own `tokio::runtime::Builder::new_current_thread()`
- `Arc<Mutex<MultiFluxState>>` shared between WS task and pollers
- `try_lock()` in pollers — skip if WS task is mid-write
- First call: initialize state, skip emission (same as WorldSignalPoller)
- Reconnect loop: on error, sleep 10s, retry

Single WS connection handles all namespaces — no need for one connection per feed.

---

## Flux Publisher

Observer-gene publishes its state to Flux every 100 ticks.
Topic: `observer-gene.state` (or configurable).
Uses the existing optional Flux bridge in `expression/engine.rs` — enable by default.
Publish is a simple HTTP POST to `POST /api/events` — no auth required for local dev.

---

## Action Space

Perception meta-actions only. No system ops (no CargoBuild, no Renice — this is not OS gene).
Actions calibrate gene's own perception; the regulation engine selects among them empirically.

| ID | Op | Target | Gate |
|----|-----|--------|------|
| 100–109 | AdjustDecay(±0.005) | key world signals | 0.50 |
| 110–119 | AdjustBaseline(±0.02) | key world signals | 0.55 |
| 120 | CoinDerivedSignal(Ratio, a, b) | top co-active pair | 0.65 |
| 121 | CoinDerivedSignal(Difference, a, b) | top co-active pair | 0.65 |
| 122 | GenAction | chronic deviations | 0.70 |
| 123 | WritePrompt | self_prompt.md | 0.60 |
| 124 | ReadPrompt | self_prompt.md | 0.50 |
| 125 | ReloadActions | actions.json | 0.50 |

Exact signal targets for AdjustDecay/AdjustBaseline chosen during Phase 5 implementation.

---

## Design Invariants (Never Violate)

1. **All world signals have weight=0.** Registered at startup, never changed. Gene cannot
   reduce imbalance by changing BTC price or aircraft count.

2. **Continuity signals are never reducible by selectable actions.** Hard filter in
   ImbalanceScorer. Do not bypass.

3. **System actions require continuity gate.** Executor checks before executing any op.

4. **Symbols carry no pre-assigned meaning.** Φ_NNNN only. No labeling or domain semantics.

5. **Checkpoint on every clean exit.** SIGTERM must save state.

6. **Nested property access is explicit in each poller.** flux_multi.rs handles one-level
   nesting generically; arrays (stocks results[]) are handled in stocks.rs directly.

7. **Gene does not write to Flux market namespaces.** It only writes to `observer-gene.state`.

8. **No modifications to gene-core logic without explicit instruction.**

---

## Build & Run

```bash
# From gene-observer/
cargo build --release

# Run headless (start here)
./target/release/observer-gene --data-dir ./observer-data --no-tui

# Run with Flux
./target/release/observer-gene \
  --data-dir ./observer-data \
  --flux-url wss://api.flux-universe.com/api/ws \
  --no-tui

# Monitor
./target/release/gene-ctl symbols
./target/release/gene-ctl expressions 20
tail -f observer-data/observer-gene.log

# Build check (fast)
cargo check
```

No automated tests. Build verification: `cargo check` or `cargo build --release`.

---

## Key References in gene-core

Read these before implementing anything. The patterns must match exactly.

| File | What to follow |
|------|---------------|
| `gene-core/src/signal/flux.rs` | WS background thread, shared state, try_lock, reconnect loop |
| `gene-core/src/signal/world.rs` | WorldSignalPoller: first-call init skip, polling pattern |
| `gene-core/src/signal/bus.rs` | `register_with_id`, `set_value`, `get_value` |
| `gene-core/src/signal/types.rs` | Signal, SignalClass::World, weight field |
| `gene-core/src/expression/engine.rs` | Flux publication bridge (enable by default) |
| `gene-core/src/main.rs` | Tick loop structure, build_bus(), action space wiring |
| `projects/gene-financial/financial-gene/src/main.rs` | How a non-OS gene binary is wired |
