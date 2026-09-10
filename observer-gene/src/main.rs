use gene_core::expression::ExpressionEngine;
use gene_core::pattern::extractor::PatternExtractor;
use gene_core::pattern::index::PatternIndex;
use gene_core::persistence::codegen::CodeGenerator;
use gene_core::persistence::selfmod::SelfModifier;
use gene_core::persistence::store::{AgentCheckpoint, SessionStore};
use gene_core::regulation::action::{Action, ActionSpace, DerivedOp, SystemOp};
use gene_core::regulation::causal::CausalTracer;
use gene_core::regulation::drive::RegulationDrive;
use gene_core::regulation::selector::ActionSelector;
use gene_core::selfmodel::evaluator::ActionEvaluator;
use gene_core::selfmodel::meta::MetaSignal;
use gene_core::selfmodel::model::SelfModel;
use gene_core::signal::bus::SignalBus;
use gene_core::signal::ledger::SignalLedger;
use gene_core::signal::types::{DeltaSource, SignalClass, SignalId};
use gene_core::symbol::activation::SymbolActivationFrame;
use gene_core::symbol::composition::CompositionEngine;
use gene_core::symbol::grounder::SymbolGrounder;
use gene_core::symbol::ledger::SymbolLedger;

mod degeneracy_tracker;
mod familiarity_tracker;
mod predict;
mod publisher;
mod registry;
mod shape;
mod signal;
mod stddev_tracker;

use degeneracy_tracker::DegeneracyTracker;
use familiarity_tracker::FamiliarityTracker;
use predict::PredictionEngine;
use stddev_tracker::StddevTracker;

use anyhow::Result;
use clap::Parser;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::registry::SignalRegistry;

/// Seed registry compiled into the binary for self-contained deployment.
/// On first run the runtime file is absent; this JSON is written to
/// `observer-data/signal-registry.json`. After that the runtime file is
/// the source of truth and this constant is never consulted again.
const SEED_REGISTRY_JSON: &str = include_str!("../seed-registry.json");

/// Catalog compiled into the binary (no hot-reload per ADR 001).
/// Defines the filter, normalization recipes, and all catalog-driven feeds.
const FEEDS_CATALOG_TOML: &str = include_str!("../feeds.toml");

use gene_core::signal::flux::{FluxEarthquakeState, FluxPoller, FluxSignalIds, spawn_flux_ws_task};
use crate::publisher::FluxPublisher;
use crate::shape::EntityFilter;
use crate::signal::flux_multi::{MultiFluxState, spawn_flux_multi_ws_task};

// ── Signal IDs (well-known, never change) ─────────────────────────────────────

// Continuity — exponential cost; near-zero = catastrophic
const SIG_CONTINUITY: SignalId = SignalId(0);
const SIG_INTEGRITY:  SignalId = SignalId(1);
const SIG_COHERENCE:  SignalId = SignalId(2);

// Derived internal
const SIG_META:               SignalId = SignalId(3);
const SIG_DRIVE:              SignalId = SignalId(4);
const SIG_PREDICTION_ERROR:   SignalId = SignalId(5);
const SIG_LEDGER_DEGENERACY:  SignalId = SignalId(6);
const SIG_FAMILIARITY:        SignalId = SignalId(7);

// Weather (IDs 10–27), Economic (IDs 140–144), Internet (ID 150) are
// catalog-driven via feeds.toml / shape module (task-07).
// Crypto (IDs 30–44, 160–189), Stocks (IDs 50–59), Commodities (IDs 130–135)
// are catalog-driven via feeds.toml / shape module (task-08).
// Aviation (IDs 70–78), Ships (IDs 90–95) are catalog-driven via feeds.toml
// / shape module (task-09) — aggregation shape.
// IDs are managed by the registry and resolved at startup in shape::build_pollers().

// Earthquakes — 4 aggregates (rate, magnitude, depth, significance) — weight=0
const SIG_QUAKE_RATE:      SignalId = SignalId(100);
const SIG_QUAKE_MAGNITUDE: SignalId = SignalId(101);
const SIG_QUAKE_DEPTH:     SignalId = SignalId(102);
const SIG_QUAKE_SIG:       SignalId = SignalId(103);

// Forgetting thresholds
const PATTERN_FORGET_TICKS:   u64 = 50_000;
const COMPOSITE_FORGET_TICKS: u64 = 100_000;

// ── CLI args ──────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "observer-gene", about = "World observation agent")]
struct Args {
    #[arg(long, default_value = "./observer-data")]
    data_dir: PathBuf,

    /// Stop after this many ticks (0 = run forever)
    #[arg(long, default_value = "0")]
    max_ticks: u64,

    /// Minimum microseconds per tick (0 = unlimited)
    #[arg(long, default_value = "1000")]
    tick_floor_us: u64,

    /// Ceiling for the ledger's on-disk footprint, in MiB, enforced against
    /// sled's reported size. Previously this was converted to an entry count via
    /// an assumed 256 bytes per snapshot, which measured ~182 KB in practice —
    /// so the ledger reached 36.9 GiB under a nominal 512 MB setting.
    #[arg(long, default_value = "512")]
    disk_quota_mb: u64,

    /// Oldest entries dropped per compaction pass once the byte quota is
    /// exceeded. Larger batches compact less often but shed more at a time.
    #[arg(long, default_value = "50000")]
    compact_batch_entries: u64,

    #[arg(long, default_value = "1000")]
    checkpoint_interval: u64,

    /// Flux WebSocket URL for live world data (empty = no Flux connection)
    #[arg(long, default_value = "")]
    flux_url: String,

    /// Bearer token for Flux HTTP publish (required when Flux auth is enabled)
    #[arg(long)]
    flux_token: Option<String>,
}

// ── Registry verification ─────────────────────────────────────────────────────

/// Verify that every `SIG_*` const matches the corresponding registry entry.
/// Panics with a clear message if any key is missing or has an unexpected ID.
/// This runs at startup after the registry is loaded; mismatches are bugs, not
/// recoverable errors.
fn verify_registry_consts(registry: &SignalRegistry) {
    // (full key string, expected SignalId u32)
    // Weather (10–27), Economic (140–144), Internet (150):   catalog-driven, verified by build_pollers.
    // Crypto (30–44, 160–189), Stocks (50–59), Commodities (130–135): catalog-driven, verified by build_pollers.
    // Aviation (70–78), Ships (90–95): catalog-driven, verified by build_pollers.
    let checks: &[(&str, u32)] = &[
        // Internal
        ("internal/continuity#value",  SIG_CONTINUITY.0),
        ("internal/integrity#value",   SIG_INTEGRITY.0),
        ("internal/coherence#value",   SIG_COHERENCE.0),
        ("internal/meta#value",               SIG_META.0),
        ("internal/drive#value",              SIG_DRIVE.0),
        ("internal/prediction_error#value",   SIG_PREDICTION_ERROR.0),
        ("internal/ledger_degeneracy#value",  SIG_LEDGER_DEGENERACY.0),
        ("internal/familiarity#value",        SIG_FAMILIARITY.0),
        // Earthquakes
        ("flux-earthquakes#agg.rate",      SIG_QUAKE_RATE.0),
        ("flux-earthquakes#agg.magnitude", SIG_QUAKE_MAGNITUDE.0),
        ("flux-earthquakes#agg.depth",     SIG_QUAKE_DEPTH.0),
        ("flux-earthquakes#agg.sig",       SIG_QUAKE_SIG.0),
    ];

    for &(key, expected) in checks {
        match registry.get_by_key(key) {
            None => panic!("signal registry missing key: {}", key),
            Some(id) if id.0 != expected => panic!(
                "signal registry key {} has ID {} but const expects {}",
                key, id.0, expected
            ),
            _ => {}
        }
    }
}

// ── Signal bus construction ───────────────────────────────────────────────────

fn build_bus() -> SignalBus {
    let mut bus = SignalBus::new();

    // Continuity — exponential penalty, near-zero = catastrophic
    bus.register_with_id(SIG_CONTINUITY, SignalClass::Continuity, 1.0, 0.0,   50.0);
    bus.register_with_id(SIG_INTEGRITY,  SignalClass::Continuity, 1.0, 0.0,   30.0);
    bus.register_with_id(SIG_COHERENCE,  SignalClass::Continuity, 1.0, 0.0,   20.0);

    // Derived internal
    bus.register_with_id(SIG_META,              SignalClass::Derived, 0.5, 0.001, 2.0);
    bus.register_with_id(SIG_DRIVE,             SignalClass::Derived, 0.0, 0.05,  1.0);
    bus.register_with_id(SIG_PREDICTION_ERROR,  SignalClass::Derived, 0.0, 0.01,  5.0);
    bus.register_with_id(SIG_LEDGER_DEGENERACY, SignalClass::Derived, 0.0, 0.02,  3.0);
    bus.register_with_id(SIG_FAMILIARITY,       SignalClass::Derived, 0.0, 0.05,  2.0);

    // Weather (10–27), Economic (140–144), Internet (150),
    // Crypto (30–44, 160–189), Stocks (50–59), Commodities (130–135),
    // Aviation (70–78), Ships (90–95):
    // all registered by shape::build_pollers() at startup (catalog-driven, task-07/08/09).

    // Earthquakes — weight=0, slow decay (seismic activity changes on hour timescales)
    for id in [SIG_QUAKE_RATE, SIG_QUAKE_MAGNITUDE, SIG_QUAKE_DEPTH, SIG_QUAKE_SIG] {
        bus.register_with_id(id, SignalClass::World, 0.0, 0.002, 0.0);
    }

    bus
}

// ── Action space ──────────────────────────────────────────────────────────────

fn build_action_space(registry: &SignalRegistry) -> ActionSpace {
    let lookup = |key: &str| -> SignalId {
        registry.get_by_key(key)
            .unwrap_or_else(|| panic!(
                "perception action target key not found in registry: {}", key
            ))
    };

    // Five hand-picked perception targets. To add a new target:
    //   1. Add a let-binding here
    //   2. Append two AdjustDecay actions (next available ID, ± delta)
    //   3. Append two AdjustBaseline actions (similar)
    let btc_price       = lookup("flux-crypto/bitcoin#price");
    let eth_price       = lookup("flux-crypto/ethereum#price");
    let spy_close       = lookup("flux-stocks/SPY#close");
    let aviation_eu_cnt = lookup("flux-aviation-europe#agg.count");
    let ships_ns_cnt    = lookup("flux-ships-north-sea#agg.count");

    let a = |id, op, gate| Action::new(id, vec![], 1, 0.0).with_system_op(op, gate);

    let actions = vec![
        // AdjustDecay — tune how fast signals track incoming data (IDs 100–109)
        a(100, SystemOp::AdjustDecay { signal_id: btc_price,       delta:  0.005 }, 0.50),
        a(101, SystemOp::AdjustDecay { signal_id: btc_price,       delta: -0.005 }, 0.50),
        a(102, SystemOp::AdjustDecay { signal_id: eth_price,       delta:  0.005 }, 0.50),
        a(103, SystemOp::AdjustDecay { signal_id: eth_price,       delta: -0.005 }, 0.50),
        a(104, SystemOp::AdjustDecay { signal_id: spy_close,       delta:  0.005 }, 0.50),
        a(105, SystemOp::AdjustDecay { signal_id: spy_close,       delta: -0.005 }, 0.50),
        a(106, SystemOp::AdjustDecay { signal_id: aviation_eu_cnt, delta:  0.005 }, 0.50),
        a(107, SystemOp::AdjustDecay { signal_id: aviation_eu_cnt, delta: -0.005 }, 0.50),
        a(108, SystemOp::AdjustDecay { signal_id: ships_ns_cnt,    delta:  0.005 }, 0.50),
        a(109, SystemOp::AdjustDecay { signal_id: ships_ns_cnt,    delta: -0.005 }, 0.50),

        // AdjustBaseline — recalibrate "normal" for key world signals (IDs 110–119)
        a(110, SystemOp::AdjustBaseline { signal_id: btc_price,       delta:  0.02 }, 0.55),
        a(111, SystemOp::AdjustBaseline { signal_id: btc_price,       delta: -0.02 }, 0.55),
        a(112, SystemOp::AdjustBaseline { signal_id: eth_price,       delta:  0.02 }, 0.55),
        a(113, SystemOp::AdjustBaseline { signal_id: eth_price,       delta: -0.02 }, 0.55),
        a(114, SystemOp::AdjustBaseline { signal_id: spy_close,       delta:  0.02 }, 0.55),
        a(115, SystemOp::AdjustBaseline { signal_id: spy_close,       delta: -0.02 }, 0.55),
        a(116, SystemOp::AdjustBaseline { signal_id: aviation_eu_cnt, delta:  0.02 }, 0.55),
        a(117, SystemOp::AdjustBaseline { signal_id: aviation_eu_cnt, delta: -0.02 }, 0.55),
        a(118, SystemOp::AdjustBaseline { signal_id: ships_ns_cnt,    delta:  0.02 }, 0.55),
        a(119, SystemOp::AdjustBaseline { signal_id: ships_ns_cnt,    delta: -0.02 }, 0.55),

        // CoinDerivedSignal — seed derived signal pairs (IDs 120–121)
        a(120, SystemOp::CoinDerivedSignal { op: DerivedOp::Ratio,      signal_a: btc_price, signal_b: eth_price }, 0.65),
        a(121, SystemOp::CoinDerivedSignal { op: DerivedOp::Difference, signal_a: btc_price, signal_b: spy_close }, 0.65),

        // Self-model and housekeeping (IDs 122–125)
        a(122, SystemOp::GenAction,     0.70),
        a(123, SystemOp::WritePrompt,   0.60),
        a(124, SystemOp::ReadPrompt,    0.50),
        a(125, SystemOp::ReloadActions, 0.50),
    ];

    let mut space = ActionSpace::new(actions);
    space.next_id = 200;
    space
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .init();

    let args = Args::parse();
    std::fs::create_dir_all(&args.data_dir)?;

    tracing::info!("observer-gene starting");

    // ── Signal registry ───────────────────────────────────────────────────────
    // On first run (runtime file absent) the seed JSON is written from the
    // compiled-in constant. Subsequent runs load the runtime file directly.
    // verify_registry_consts() panics loudly if any SIG_* const diverges from
    // the registry — catches seed authoring errors at startup, not silently.
    // Mutable so shape::build_pollers can call register() for catalog signals.
    let registry_path = args.data_dir.join("signal-registry.json");
    let mut registry = SignalRegistry::load_or_seed(&registry_path, SEED_REGISTRY_JSON)?;
    verify_registry_consts(&registry);

    // ── Catalog ───────────────────────────────────────────────────────────────
    // Compiled into the binary via include_str! — no hot-reload.
    let catalog = shape::parse_catalog(FEEDS_CATALOG_TOML)?;

    let ledger_path   = args.data_dir.join("ledger");
    // Byte quota, enforced directly. The old form divided by an assumed 256
    // bytes per snapshot; measured snapshots are ~182 KB, so "512 MB" was
    // authorising tens of gigabytes.
    let quota_bytes = args.disk_quota_mb * 1024 * 1024;

    let mut bus = build_bus();

    // ── Flux WebSocket tasks ──────────────────────────────────────────────────
    // MultiFluxState now carries the catalog filter (replaces hardcoded is_handled).
    let filter = EntityFilter::from_config(&catalog.filter);
    let multi_state = Arc::new(Mutex::new(MultiFluxState::with_filter(filter)));
    let quake_state = Arc::new(Mutex::new(FluxEarthquakeState::new()));

    let state_publisher: Option<FluxPublisher> = if !args.flux_url.is_empty() {
        spawn_flux_multi_ws_task(args.flux_url.clone(), multi_state.clone());
        spawn_flux_ws_task(args.flux_url.clone(), quake_state.clone());
        tracing::info!("flux ws tasks spawned: {}", args.flux_url);
        let http_base = ws_to_http_base(&args.flux_url);
        tracing::info!("flux publisher → {}/api/events", http_base);
        let pub_ = FluxPublisher::new(http_base, args.flux_token.clone());
        pub_.publish_key(&registry);
        Some(pub_)
    } else {
        tracing::info!("no --flux-url; world signals will hold baseline");
        None
    };

    // ── Catalog-driven shape pollers (weather, economic, internet) ────────────
    // Registers these signals on the bus (IDs come from registry, guaranteed
    // to match seed; panics on mismatch). Emits startup log per feed.
    let mut shape_pollers = shape::build_pollers(
        &catalog, &mut registry, multi_state.clone(), &mut bus,
    )?;

    // ── Per-feed pollers (remaining: earthquakes — stays hardcoded per ADR §3) ──
    // Aviation (task-09) and ships (task-09) are now catalog-driven via
    // shape_pollers above (aggregation shape).
    let quake_ids = FluxSignalIds {
        quake_rate:      SIG_QUAKE_RATE,
        quake_magnitude: SIG_QUAKE_MAGNITUDE,
        quake_depth:     SIG_QUAKE_DEPTH,
        quake_sig:       SIG_QUAKE_SIG,
    };
    let mut quake_poller = FluxPoller::new(quake_state);

    let signal_baselines: std::collections::HashMap<SignalId, f64> =
        bus.all_signals().iter().map(|(id, s)| (*id, s.baseline)).collect();

    let mut action_space      = build_action_space(&registry);
    let mut ledger            = SignalLedger::open(
        &ledger_path, quota_bytes, args.compact_batch_entries)?;
    let mut causal            = CausalTracer::new(10_000);
    let mut pattern_extractor = PatternExtractor::new();
    let mut pattern_index     = PatternIndex::new();
    let mut symbol_ledger     = SymbolLedger::new();
    let     symbol_grounder   = SymbolGrounder::new(0.05);
    let mut self_model        = SelfModel::new(2000, 10);
    let mut meta              = MetaSignal::new(SIG_META);
    let     evaluator         = ActionEvaluator::new();
    let mut drive             = RegulationDrive::new(SIG_DRIVE, 50);
    let mut selector          = ActionSelector::new(0.15, 30);
    let mut expr_engine       = ExpressionEngine::new(&args.data_dir, 100);
    let mut composition_engine = CompositionEngine::new(20);
    let mut stddev_tracker       = StddevTracker::new();
    let mut familiarity_tracker  = FamiliarityTracker::new();
    let mut degeneracy_tracker   = DegeneracyTracker::new();
    let mut prediction_engine    = PredictionEngine::new();

    let store         = SessionStore::new(&args.data_dir);
    let mut self_modifier = SelfModifier::new(
        SessionStore::new(&args.data_dir),
        std::env::current_dir()?,
    );
    // Observer-gene runs indefinitely with slow-changing signals; 500-tick default
    // causes constant rewrite spam. 100k ticks ≈ 100s between directive updates.
    self_modifier.directive_mod_interval = 100_000;

    let mut derived_signals: Vec<(SignalId, DerivedOp, SignalId, SignalId)> = Vec::new();

    let mut tick: u64 = 0;

    // Load checkpoint
    match store.load() {
        Ok(Some(cp)) => {
            tick = cp.tick;
            for (id, val) in &cp.signal_values {
                bus.set_value(*id, *val);
            }
            causal         = cp.causal_tracer;
            pattern_index  = cp.pattern_index;
            symbol_ledger  = cp.symbol_ledger;
            self_model     = cp.self_model;
            composition_engine.seed_from_ledger(&symbol_ledger);
            tracing::info!("resumed from tick {}", tick);
        }
        Ok(None)   => tracing::info!("no checkpoint found, starting fresh"),
        Err(e)     => tracing::warn!("checkpoint load failed, starting fresh: {}", e),
    }

    // Load persisted actions
    let actions_json_path  = args.data_dir.join("actions.json");
    let mut actions_json_mtime = get_mtime(&actions_json_path);
    if actions_json_path.exists() {
        if let Ok(json) = std::fs::read_to_string(&actions_json_path) {
            let added = action_space.merge_from_json(&json).unwrap_or(0);
            if added > 0 {
                tracing::info!("loaded {} persisted actions", added);
            }
        }
    }

    // Graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).ok();

    tracing::info!("entering tick loop at tick {}", tick);

    let mut imbalance_history: VecDeque<f64> = VecDeque::with_capacity(20);

    loop {
        let tick_start = Instant::now();

        if !running.load(Ordering::SeqCst) {
            tracing::info!("shutdown requested at tick {}", tick);
            break;
        }
        if args.max_ticks > 0 && tick >= args.max_ticks {
            tracing::info!("max ticks {} reached", args.max_ticks);
            break;
        }

        // Hot-reload actions.json
        if tick % 100 == 0 {
            let new_mtime = get_mtime(&actions_json_path);
            if new_mtime != actions_json_mtime {
                if let Ok(json) = std::fs::read_to_string(&actions_json_path) {
                    let added = action_space.merge_from_json(&json).unwrap_or(0);
                    if added > 0 {
                        tracing::info!("hot-reloaded {} new actions", added);
                    }
                }
                actions_json_mtime = new_mtime;
            }
        }

        // ── Layer 0: Signal tick ──────────────────────────────────────────────
        let (imbalance, snapshot) = bus.tick(tick);
        let pre_snapshot = snapshot.clone();
        ledger.append(&snapshot)?;
        pattern_extractor.push(snapshot);

        // ── Signal polls ─────────────────────────────────────────────────────
        if tick % 10 == 0 {
            // Catalog-driven shape pollers (weather, economic, internet, crypto,
            // stocks, commodities, aviation, ships — all via feeds.toml)
            for p in shape_pollers.iter_mut() { p.poll(&mut bus); }
            quake_poller.poll(&mut bus, &quake_ids);
        }

        // ── Self-continuity: operational health metrics ───────────────────────
        // Updated every 100 ticks so the expression record reflects current state.
        // s_continuity — runtime establishment: ramps from 0.8 → 1.0 over 500k ticks.
        //   Represents: how long has this observer been running continuously?
        // s_integrity  — live world signal coverage: fraction of world signals with
        //   |value−baseline| > 0.05, indicating real data is flowing through them.
        //   Directly tracks feed health (e.g. aviation down → integrity falls).
        // s_coherence  — symbol formation depth: grows as patterns emerge, floor 0.75.
        //   Represents: is the pattern-recognition machinery actually working?
        if tick % 100 == 0 {
            let c = (0.8 + 0.2 * (tick as f64 / 500_000.0)).min(1.0);
            bus.set_value(SIG_CONTINUITY, c);

            let (total_world, active_world) = {
                let sigs = bus.all_signals();
                let total  = sigs.values().filter(|s| s.class == SignalClass::World).count();
                let active = sigs.values()
                    .filter(|s| s.class == SignalClass::World)
                    .filter(|s| {
                        // Use original registration baseline (immune to AdjustBaseline actions)
                        let orig = signal_baselines.get(&s.id).copied().unwrap_or(0.5);
                        (s.value - orig).abs() > 0.05
                    })
                    .count();
                (total, active)
            };
            let i = if total_world > 0 {
                (0.5 + 0.5 * active_world as f64 / total_world as f64).min(1.0)
            } else {
                0.5
            };
            bus.set_value(SIG_INTEGRITY, i);

            let co = (0.75 + 0.25 * (symbol_ledger.len() as f64 / 40.0)).min(1.0);
            bus.set_value(SIG_COHERENCE, co);
        }

        // ── Layer 1: Regulation ───────────────────────────────────────────────
        let urgency = drive.urgency(imbalance, 2000.0);
        bus.set_value(SIG_DRIVE, urgency);

        let circuit_break = drive.update(imbalance, tick);
        if circuit_break {
            tracing::warn!("tick {}: stagnation — forcing exploration", tick);
            drive.reset_stagnation();
            selector = ActionSelector::new(0.9, 30);
        }

        let active_thresholds = stddev_tracker.thresholds();
        let active_signals = pattern_extractor.current_active(
            &active_thresholds,
            &signal_baselines,
            0.02,  // fallback for un-tracked signals
        );

        // ── Familiarity tracking (SIG_FAMILIARITY = ID 7) ────────────────────
        let fam_pattern_id = FamiliarityTracker::pattern_id(&active_signals);
        familiarity_tracker.observe(fam_pattern_id);
        let fam = familiarity_tracker.familiarity(fam_pattern_id);
        bus.set_value(SIG_FAMILIARITY, fam);

        // ── Layer 3: Symbol activation ────────────────────────────────────────
        symbol_grounder.process_salience(&pattern_index, &mut symbol_ledger, tick);
        let _ = symbol_grounder.update_activations(
            &pattern_index, &mut symbol_ledger, &active_signals, imbalance, tick,
        );
        let frame = SymbolActivationFrame::build(tick, &symbol_ledger, 0.1);

        // ── Ledger degeneracy tracking (SIG_LEDGER_DEGENERACY = ID 6) ─────────
        let active_indices: Vec<u32> = frame.active.iter().map(|(idx, _, _)| *idx).collect();
        degeneracy_tracker.observe(active_indices);
        let degeneracy = degeneracy_tracker.degeneracy();
        bus.set_value(SIG_LEDGER_DEGENERACY, degeneracy);

        // ── Symbol composition ────────────────────────────────────────────────
        composition_engine.observe(&frame.active, &symbol_ledger);
        if tick % 4 == 0 {
            let new_composites = composition_engine.maybe_compose(&mut symbol_ledger, tick);
            if !new_composites.is_empty() {
                tracing::info!("tick {}: coined {} composite symbol(s)", tick, new_composites.len());
            }
        }

        // ── Layer 4: Select action ────────────────────────────────────────────
        // Read the latest steer command (single-use, cleared on read).
        // Validate: confidence ≥ 0.80, tick_ref not more than 100,000 ticks stale.
        let steered_action: Option<u32> = if let Some(cmd) = take_steer_command(&multi_state) {
            let stale = tick.saturating_sub(cmd.tick_ref) > 100_000;
            if stale {
                tracing::info!(
                    "tick {}: steer ignored — stale (tick_ref={}, current={})",
                    tick, cmd.tick_ref, tick
                );
                None
            } else if cmd.confidence < 0.80 {
                tracing::info!(
                    "tick {}: steer ignored — confidence {:.2} < 0.80",
                    tick, cmd.confidence
                );
                None
            } else {
                // Pick the first action_id from the list
                cmd.action_ids.into_iter().next().map(|id| {
                    tracing::info!(
                        "tick {}: steer candidate action={} confidence={:.2} reason=\"{}\"",
                        tick, id, cmd.confidence, cmd.reason
                    );
                    id
                })
            }
        } else {
            None
        };

        let chosen_action_id = evaluator.select(
            &bus, &action_space, &causal, &mut selector,
            &self_model, &meta, &frame, urgency, tick, steered_action,
        );

        // Log when a steered action was used or overridden
        if let Some(steer_id) = steered_action {
            if chosen_action_id == Some(steer_id) {
                tracing::info!("tick {}: steered action {} accepted", tick, steer_id);
            } else {
                tracing::info!(
                    "tick {}: steered action {} overridden by regulation (chose {:?})",
                    tick, steer_id, chosen_action_id
                );
            }
        }

        // ── Execute chosen action ─────────────────────────────────────────────
        if let Some(action_id) = chosen_action_id {
            let (effects, system_op, gate) = match action_space.get(action_id) {
                Some(a) => (
                    a.effect_profile.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
                    a.system_op.clone(),
                    a.continuity_gate,
                ),
                None => (vec![], None, 0.0),
            };

            for (sig_id, delta) in effects {
                bus.queue_delta(sig_id, delta, DeltaSource::Action(action_id));
            }

            if let Some(op) = system_op {
                exec_perception_op(
                    &op, gate, &mut bus, &mut action_space, &mut derived_signals,
                    &causal, &self_model, &symbol_ledger, &frame, tick, &args.data_dir,
                );
            }
        }

        // ── Second bus tick ───────────────────────────────────────────────────
        let (post_imbalance, post_snapshot) = bus.tick(tick);

        // Feed post-tick values into the stddev tracker for next-tick thresholds.
        for (id, value) in post_snapshot.values.iter() {
            stddev_tracker.observe(*id, *value);
        }

        // Update per-signal EMA predictors; score matured predictions; aggregate error.
        prediction_engine.tick(tick, &post_snapshot.values);
        let pred_err = prediction_engine.aggregate_error();
        bus.set_value(SIG_PREDICTION_ERROR, pred_err);

        imbalance_history.push_back(post_imbalance);
        if imbalance_history.len() > 20 { imbalance_history.pop_front(); }

        // ── Derived signal computation ────────────────────────────────────────
        for (derived_id, op, sig_a, sig_b) in &derived_signals {
            let a = bus.get_value(*sig_a);
            let b = bus.get_value(*sig_b);
            let val = match op {
                DerivedOp::Ratio      => (a / b.max(1e-9)).clamp(0.0, 1.0),
                DerivedOp::Difference => ((a - b + 1.0) / 2.0).clamp(0.0, 1.0),
                DerivedOp::Product    => (a * b).clamp(0.0, 1.0),
            };
            bus.set_value(*derived_id, val);
        }

        // ── Layer 2: Pattern extraction ───────────────────────────────────────
        if tick % 4 == 0 {
            if let Some(result) = pattern_extractor.extract(&signal_baselines) {
                pattern_index.integrate(result, chosen_action_id);
            }
        }

        // ── Layer 4: Causal + self-model ──────────────────────────────────────
        if let Some(action_id) = chosen_action_id {
            causal.record(action_id, tick, &pre_snapshot, &post_snapshot);
        }

        meta.update(imbalance, post_imbalance);
        bus.set_value(SIG_META, meta.bus_value());

        let imbalance_delta = post_imbalance - imbalance;
        self_model.update(tick, &frame, post_imbalance, chosen_action_id, imbalance_delta);

        // ── Expression layer + Flux publish ───────────────────────────────────
        // maybe_emit fires every 100 ticks (local log); publish to Flux every 10,000 ticks (~10s)
        if let Some(record) = expr_engine.maybe_emit(
            tick, &frame, &bus, &self_model, &meta, chosen_action_id, &causal,
        ) {
            if tick % 10_000 == 0 {
                if let Some(pub_) = &state_publisher {
                    pub_.publish(record);

                    // Build and publish symbol composition map
                    let mut sym_map = serde_json::Map::new();
                    for sym in symbol_ledger.all() {
                        let components: Vec<serde_json::Value> = if sym.is_composite {
                            sym.parents.iter()
                                .map(|&idx| serde_json::Value::String(format!("Φ_{:04}", idx)))
                                .collect()
                        } else {
                            sym.signal_cluster.iter()
                                .map(|id| serde_json::Value::String(format!("s_{:04}", id.0)))
                                .collect()
                        };
                        sym_map.insert(sym.token.clone(), serde_json::Value::Array(components));
                    }
                    pub_.publish_symbols(serde_json::Value::Object(sym_map));
                    pub_.publish_key_async(&registry);
                }
            }
        }

        // ── Layer 5: Periodic self-modification ───────────────────────────────
        if tick > 0 && tick % self_modifier.directive_mod_interval == 0 {
            if let Err(e) = self_modifier.rewrite_directives(
                tick, &self_model, &symbol_ledger, &frame, &causal,
            ) {
                tracing::warn!("directive rewrite failed: {}", e);
            }
        }

        // ── Checkpoint ────────────────────────────────────────────────────────
        if tick > 0 && tick % args.checkpoint_interval == 0 {
            let cp = AgentCheckpoint {
                tick,
                signal_values: bus.snapshot_values(),
                causal_tracer: causal.clone_partial(),
                pattern_index: pattern_index.clone(),
                symbol_ledger: symbol_ledger.clone(),
                self_model:    self_model.clone(),
                action_imbalance_history: Vec::new(),
            };
            if let Err(e) = store.save(&cp) {
                tracing::warn!("checkpoint failed: {}", e);
            } else {
                tracing::info!("tick {}: checkpoint saved", tick);
            }
            ledger.flush()?;
        }

        // ── Forgetting / pruning ──────────────────────────────────────────────
        if tick > 0 && tick % 5000 == 0 {
            let protected: std::collections::HashSet<u64> = symbol_ledger.all()
                .filter(|s| !s.is_composite && s.pattern_id != 0)
                .map(|s| s.pattern_id)
                .collect();
            let pruned_patterns   = pattern_index.prune_stale(tick, PATTERN_FORGET_TICKS, &protected);
            let pruned_composites = symbol_ledger.prune_composites(tick, COMPOSITE_FORGET_TICKS);
            if !pruned_composites.is_empty() {
                composition_engine.purge_symbols(&pruned_composites);
            }
            if pruned_patterns > 0 || !pruned_composites.is_empty() {
                tracing::info!(
                    "tick {}: pruned {} pattern(s), {} composite(s)",
                    tick, pruned_patterns, pruned_composites.len()
                );
            }
        }

        // ── Periodic status log ───────────────────────────────────────────────
        if tick % 1000 == 0 {
            tracing::info!(
                "tick={} imbalance={:.3} patterns={} symbols={} action={:?}",
                tick, post_imbalance,
                pattern_index.len(), symbol_ledger.len(),
                chosen_action_id
            );
        }

        // ── Throttle ─────────────────────────────────────────────────────────
        if args.tick_floor_us > 0 {
            let elapsed = tick_start.elapsed();
            let floor   = Duration::from_micros(args.tick_floor_us);
            if elapsed < floor {
                std::thread::sleep(floor - elapsed);
            }
        }

        tick += 1;
    }

    // Final checkpoint
    let cp = AgentCheckpoint {
        tick,
        signal_values: bus.snapshot_values(),
        causal_tracer: causal.clone_partial(),
        pattern_index: pattern_index.clone(),
        symbol_ledger: symbol_ledger.clone(),
        self_model:    self_model.clone(),
        action_imbalance_history: Vec::new(),
    };
    store.save(&cp)?;
    ledger.flush()?;
    tracing::info!("observer-gene exited cleanly at tick {}", tick);
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Derive the HTTP base URL from a WebSocket URL.
/// "ws://host:port/api/ws" → "http://host:port"
/// "wss://host:port/api/ws" → "https://host:port"
fn ws_to_http_base(ws_url: &str) -> String {
    let http = if ws_url.starts_with("wss://") {
        ws_url.replacen("wss://", "https://", 1)
    } else {
        ws_url.replacen("ws://", "http://", 1)
    };
    http.trim_end_matches("/api/ws").to_string()
}

fn get_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Dispatch a perception meta-action. Called from the tick loop after effect
/// profile deltas have already been queued on the bus.
#[allow(clippy::too_many_arguments)]
fn exec_perception_op(
    op:              &SystemOp,
    gate:            f64,
    bus:             &mut SignalBus,
    action_space:    &mut ActionSpace,
    derived_signals: &mut Vec<(SignalId, DerivedOp, SignalId, SignalId)>,
    causal:          &gene_core::regulation::causal::CausalTracer,
    self_model:      &gene_core::selfmodel::model::SelfModel,
    symbol_ledger:   &SymbolLedger,
    frame:           &SymbolActivationFrame,
    tick:            u64,
    data_dir:        &std::path::Path,
) {
    let continuity = bus.get_value(SIG_CONTINUITY);
    if continuity < gate {
        tracing::warn!(
            "tick {}: {:?} blocked by continuity gate ({:.3} < {:.3})",
            tick, op, continuity, gate
        );
        return;
    }

    match op {
        SystemOp::AdjustDecay { signal_id, delta } => {
            let current = bus.get(*signal_id).map(|s| s.decay_rate).unwrap_or(0.0);
            let new_rate = (current + delta).clamp(0.0, 0.5);
            bus.set_decay_rate(*signal_id, new_rate);
            tracing::info!("tick {}: AdjustDecay {} → {:.4}", tick, signal_id, new_rate);
        }

        SystemOp::AdjustBaseline { signal_id, delta } => {
            let current = bus.get(*signal_id).map(|s| s.baseline).unwrap_or(0.0);
            let new_baseline = (current + delta).clamp(0.0, 1.0);
            bus.set_baseline(*signal_id, new_baseline);
            tracing::info!("tick {}: AdjustBaseline {} → {:.4}", tick, signal_id, new_baseline);
        }

        SystemOp::CoinDerivedSignal { op: derived_op, signal_a, signal_b } => {
            let new_id = bus.register(SignalClass::Derived, 0.5, 0.01, 0.0);
            derived_signals.push((new_id, derived_op.clone(), *signal_a, *signal_b));
            tracing::info!(
                "tick {}: CoinDerivedSignal {:?}({}, {}) → {}",
                tick, derived_op, signal_a, signal_b, new_id
            );
        }

        SystemOp::GenAction => {
            let codegen = CodeGenerator::new(data_dir.to_path_buf(), data_dir.to_path_buf());
            if let Some(action) = codegen.generate_corrective_action(bus, causal, action_space.next_id) {
                let label = action.label.clone().unwrap_or_default();
                let _ = append_action_to_file(&action, data_dir);
                let id = action_space.add(action);
                tracing::info!("tick {}: GenAction coined action {} — {}", tick, id, label);
            }
        }

        SystemOp::WritePrompt => {
            let codegen = CodeGenerator::new(data_dir.to_path_buf(), data_dir.to_path_buf());
            let content = codegen.generate_self_prompt(
                tick, bus, causal, self_model, symbol_ledger, frame,
            );
            let path = data_dir.join("self_prompt.md");
            match std::fs::write(&path, &content) {
                Ok(_)  => tracing::info!("tick {}: WritePrompt ({} bytes)", tick, content.len()),
                Err(e) => tracing::warn!("tick {}: WritePrompt failed: {}", tick, e),
            }
        }

        SystemOp::ReadPrompt => {
            let path = data_dir.join("self_prompt.md");
            match std::fs::read_to_string(&path) {
                Ok(c)  => tracing::info!("tick {}: ReadPrompt ({} bytes)", tick, c.len()),
                Err(e) => tracing::warn!("tick {}: ReadPrompt failed: {}", tick, e),
            }
        }

        SystemOp::ReloadActions => {
            tracing::info!("tick {}: ReloadActions signaled", tick);
        }

        _ => {
            // OS-specific ops and portfolio ops do not apply to observer-gene
            tracing::debug!("tick {}: system op not applicable in observer-gene", tick);
        }
    }
}

fn append_action_to_file(
    action:   &gene_core::regulation::action::Action,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let path = data_dir.join("actions.json");
    let mut actions: Vec<gene_core::regulation::action::Action> = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?).unwrap_or_default()
    } else {
        Vec::new()
    };
    if !actions.iter().any(|a| a.id == action.id) {
        actions.push(action.clone());
        std::fs::write(&path, serde_json::to_string_pretty(&actions)?)?;
    }
    Ok(())
}

// ── Steering ──────────────────────────────────────────────────────────────────

/// Parsed steer command received from knowledge-gene/steer.
struct SteerCommand {
    action_ids: Vec<u32>,
    confidence: f64,
    tick_ref:   u64,
    reason:     String,
}

/// Read the latest steer command from MultiFluxState and clear it (single-use).
/// Returns None if no command is present or if fields are malformed.
fn take_steer_command(state: &Arc<Mutex<MultiFluxState>>) -> Option<SteerCommand> {
    let mut s = state.try_lock().ok()?;
    let props = s.entities.get("knowledge-gene/steer")?;

    // Entity is present — log which fields we currently see. Helps diagnose
    // task-02 partial-entity recoveries (caller will retry next tick) and
    // any field-type mismatches.
    let field_names: Vec<&str> = props.keys().map(|k| k.as_str()).collect();
    tracing::info!("steer entity present: fields={:?}", field_names);

    // Require action_ids, confidence, tick_ref to all be present and
    // correctly typed before treating this as a complete command. Flux
    // delivers per-property state_updates, so an in-flight publish can leave
    // the entity partially populated. Reading a partial command and then
    // removing the entity (below) would lose the fields that arrive next.
    // reason is informational and treated as optional.
    let action_ids: Vec<u32> = props.get("action_ids")?
        .as_array()?
        .iter()
        .filter_map(|x| x.as_u64().map(|n| n as u32))
        .collect();
    if action_ids.is_empty() {
        return None;
    }
    let confidence = props.get("confidence")?.as_f64()?;
    let tick_ref   = props.get("tick_ref")?.as_u64()?;
    let reason     = props.get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // All required fields present — safe to consume.
    s.entities.remove("knowledge-gene/steer");

    Some(SteerCommand { action_ids, confidence, tick_ref, reason })
}
