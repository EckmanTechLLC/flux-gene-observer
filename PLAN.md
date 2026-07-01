# observer-gene — Build Plan

Concise phase list. Each phase is a discrete session with a clear deliverable.
See CLAUDE.md for data shapes, signal IDs, and design invariants.
See OBSERVER_GENE.md for design rationale.

---

## Phase 1 — Repo Bootstrap

**Deliverable:** workspace compiles with a minimal main.rs

- [ ] Create workspace `Cargo.toml` (members: `["gene-core", "observer-gene"]`)
- [ ] Copy `gene-core/` from `gene-financial/gene-core` (already has lib target)
- [ ] Create `observer-gene/Cargo.toml` (path dep on `../gene-core`)
- [ ] Write minimal `main.rs` — prints "observer-gene starting" and exits
- [ ] Verify `cargo check` passes

---

## Phase 2 — Config & Signal Registration

**Deliverable:** all signals registered on the bus; gene starts, ticks, and exits cleanly

- [ ] Write `config.rs`: `Config` struct (TOML), load from file or CLI arg
  - Fields: `flux_url`, `flux_state_topic`, `tick_floor_us`, `max_ticks`
- [ ] Write `build_bus()` in `main.rs`: register all signals per CLAUDE.md signal ID table
  - IDs 0–4: continuity + internal (same as financial-gene)
  - IDs 10–27: weather (6 cities × 3 signals)
  - IDs 30–44: crypto (5 assets × 3 signals)
  - IDs 50–59: stocks (5 tickers × 2 signals)
  - IDs 70–78: aviation (3 zones × 3 aggregates)
  - IDs 90–95: ships (3 zones × 2 aggregates)
  - IDs 100–103: earthquakes
- [ ] Wire tick loop (copy structure from gene-financial/main.rs)
- [ ] Verify `cargo build --release` passes; binary starts and ticks

---

## Phase 3 — flux_multi.rs

**Deliverable:** single WS connection subscribing to all Flux namespaces; shared state populated

- [ ] Define `MultiFluxState`: per-namespace entity maps (HashMap<String, HashMap<String, Value>>)
- [ ] Implement `spawn_flux_multi_ws_task(url, state)` — background OS thread + tokio runtime
- [ ] `connect_and_listen`: subscribe `{"type":"subscribe","entity_id":"*"}`, route by entity_id prefix
- [ ] `handle_message`: `state_update` → update entity property in correct namespace bucket
- [ ] `entity_deleted` → remove from bucket
- [ ] Reconnect loop (10s sleep on error — same as flux.rs)
- [ ] Export `MultiFluxState` to pollers via `Arc<Mutex<MultiFluxState>>`
- [ ] Verify: connect to live Flux, log entity counts per namespace after 30s

---

## Phase 4 — Per-Feed Pollers

**Deliverable:** all world signals receiving live values from Flux; gene-ctl signals shows non-zero

- [ ] `earthquakes.rs`: move existing gene-core FluxPoller logic here; wire to IDs 100–103
- [ ] `weather.rs`: WeatherPoller — extract `current.temperature_2m`, `.wind_speed_10m`,
  `.relative_humidity_2m` per city; normalize and write IDs 10–27
- [ ] `crypto.rs`: CryptoPoller — extract per-asset `usd`, `usd_24h_vol`, `usd_24h_change`
  (handle CoinGecko ID mapping: bnb→binancecoin, xrp→ripple); normalize IDs 30–44;
  compute EMA(9), EMA(21), delta in poller state
- [ ] `stocks.rs`: StocksPoller — extract `results[0].c`, `results[0].v` per ticker;
  normalize with rolling window; compute EMA, delta for IDs 50–59 + derived IDs 110+
- [ ] `aviation.rs`: AviationPoller — per zone: count entities, mean(speed_ms),
  mean(altitude_m); normalize IDs 70–78
- [ ] `ships.rs`: ShipsPoller — per zone: count entities, mean(speed);
  normalize IDs 90–95
- [ ] All pollers: `try_lock()`, first-call init skip, 95/5 EMA smoothing on output

---

## Phase 5 — Expression Engine + Flux Publisher

**Deliverable:** gene publishes state snapshots to Flux; gene-ctl expressions works

- [ ] Enable Flux publisher bridge in `expression/engine.rs` (already exists, make it default-on)
- [ ] Configure publish topic (`observer-gene.state`), publish every 100 ticks
- [ ] Verify with: `curl http://192.168.50.107:3000/api/state/entities?namespace=observer-gene`
- [ ] Verify `gene-ctl expressions 20` returns populated output

---

## Phase 6 — Action Space + Deploy

**Deliverable:** observer-gene running on VM against live feeds; symbol ledger growing

- [ ] Wire perception meta-actions: AdjustDecay/AdjustBaseline for key signals (crypto, stocks)
- [ ] Wire CoinDerivedSignal, GenAction, WritePrompt/ReadPrompt, ReloadActions
- [ ] Build release binary
- [ ] rsync to gene VM: `rsync -av --exclude 'target/' --exclude 'observer-data/' --exclude '.git/' /home/etl/projects/gene-observer/ etl@192.168.1.40:/home/etl/observer-gene/`
- [ ] Run headless on VM: `./target/release/observer-gene --data-dir ./observer-data --no-tui`
- [ ] Monitor: `gene-ctl signals`, `gene-ctl symbols` — watch symbol ledger grow
- [ ] After 24h+: inspect composite symbols (`Φ_C_*`) for cross-domain patterns

---

## Future (not in build plan)

- flux-universe.com integration (Layer 3 human-facing translation)
- LLM-assisted symbol interpretation
- Additional feeds (energy, commodities, satellite — when available in Flux)
