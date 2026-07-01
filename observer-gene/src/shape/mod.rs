//! Shape registry — catalog-driven feed model for observer-gene.
//!
//! This module owns:
//!   - The `ShapePoller` trait (called every 10 ticks from the tick loop)
//!   - Catalog TOML deserialization types
//!   - `FilterConfig` / `EntityFilter` — used by `MultiFluxState` to route WS messages
//!   - `parse_catalog()` / `build_pollers()` — startup wiring
//!
//! Shapes: `scalar`, `flat_multi`, `nested_by_key`, `array_record`, `time_series`, `aggregation`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use gene_core::signal::bus::SignalBus;
use gene_core::signal::types::{SignalClass, SignalId};
use serde::Deserialize;

use crate::registry::SignalRegistry;
use crate::signal::flux_multi::MultiFluxState;

pub mod aggregation;
pub mod array_record;
pub mod derive;
pub mod flat_multi;
pub mod nested_by_key;
pub mod normalize;
pub mod path;
pub mod scalar;
pub mod time_series;

pub use normalize::NormalizeRecipe;

// ── ShapePoller trait ────────────────────────────────────────────────────────

/// Implemented by each shape driver. Called every 10 ticks from the tick loop.
pub trait ShapePoller {
    /// Update bus signals from the latest MultiFluxState snapshot.
    /// First call initializes (skip emission) — same pattern as existing pollers.
    fn poll(&mut self, bus: &mut SignalBus);
    fn shape_name(&self) -> &'static str;
    fn signal_count(&self) -> usize;
}

// ── Catalog deserialization types ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct Catalog {
    pub filter:    FilterConfig,
    pub normalize: HashMap<String, NormalizeRecipe>,
    pub feed:      Vec<FeedConfig>,
}

/// `[filter]` section — controls which Flux entity IDs are stored in MultiFluxState.
#[derive(Deserialize, Clone)]
pub struct FilterConfig {
    #[serde(default)] pub include_prefixes: Vec<String>,
    #[serde(default)] pub exclude_prefixes: Vec<String>,
    #[serde(default)] pub include_exact:    Vec<String>,
}

/// A normalize value that is either a named recipe (`"aviation_speed"`) or an
/// inline `LinearRange` table (`{ type = "linear_range", min = 0.0, max = 300.0 }`).
/// Used in `AggregateConfig` so per-zone count divisors can be inlined.
#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum NormalizeRef {
    Named(String),
    Inline(NormalizeRecipe),
}

/// One `[[feed.aggregate]]` entry inside an `aggregation` feed.
/// Fields are accessed only within this module's `build_pollers`.
#[derive(Deserialize, Clone)]
pub struct AggregateConfig {
    /// Aggregate name: `"count"` or `"mean.<property>"`.
    pub name: String,
    /// Normalization: inline recipe or named reference.
    normalize: NormalizeRef,
}

/// One `[[feed]]` entry. Uses a flat struct to avoid TOML tagged-enum issues.
/// Fields are optional and shape-dispatched in `build_pollers`.
#[derive(Deserialize)]
pub struct FeedConfig {
    pub namespace: String,
    pub shape:     String,

    // flat_multi / array_record / time_series: plain list of entity slugs
    #[serde(default)]
    pub entities: Vec<String>,

    // flat_multi + array_record: property definitions
    #[serde(default)]
    pub property: Vec<FeedProperty>,

    // scalar + nested_by_key: per-entity config (uses [[feed.entity]] TOML key)
    #[serde(default, rename = "entity")]
    pub entity_list: Vec<FeedEntityDef>,

    // nested_by_key: signal template definitions (uses [[feed.signal]] TOML key)
    #[serde(default, rename = "signal")]
    pub signal_list: Vec<NestedSignalDef>,

    // time_series: feed-level signal config
    #[serde(default)]
    pub signal_name: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub parse_string: bool,
    #[serde(default)]
    pub positive: bool,
    #[serde(default)]
    pub normalize: Option<String>,

    // aggregation: per-aggregate definitions
    #[serde(default)]
    pub aggregate: Vec<AggregateConfig>,
}

/// Property in a `flat_multi` or `array_record` feed.
/// `name` is optional for `flat_multi` (path is used as the key suffix).
/// `name` is required for `array_record`.
#[derive(Deserialize, Clone)]
pub struct FeedProperty {
    #[serde(default)]
    pub name:         Option<String>,
    pub path:         String,
    #[serde(default)]
    pub positive:     bool,
    #[serde(default)]
    pub parse_string: bool,
    pub normalize:    String,
}

/// Entity definition for `scalar` (has path+normalize) or `nested_by_key` (has key).
#[derive(Deserialize, Clone)]
pub struct FeedEntityDef {
    pub name:         String,
    // nested_by_key
    #[serde(default)]
    pub key:          Option<String>,
    // scalar
    #[serde(default)]
    pub path:         Option<String>,
    #[serde(default)]
    pub normalize:    Option<String>,
    #[serde(default)]
    pub parse_string: bool,
}

/// Signal template in a `nested_by_key` feed.
#[derive(Deserialize, Clone)]
pub struct NestedSignalDef {
    pub name:         String,
    // extraction (mutually exclusive with derive)
    #[serde(default)]
    pub path:         Option<String>,
    #[serde(default)]
    pub parse_string: bool,
    #[serde(default)]
    pub positive:     bool,
    // derivation (mutually exclusive with path)
    #[serde(default)]
    pub derive:       Option<String>,
    #[serde(default)]
    pub from_path:    Option<String>,
    #[serde(default)]
    pub ref_path:     Option<String>,
    // normalization
    pub normalize:    String,
}

// ── EntityFilter ─────────────────────────────────────────────────────────────

/// Config-driven predicate that decides which Flux entity IDs are stored in
/// `MultiFluxState`. Replaces the hardcoded `is_handled()` function.
pub struct EntityFilter {
    pub include_prefixes: Vec<String>,
    pub exclude_prefixes: Vec<String>,
    pub include_exact:    HashSet<String>,
}

impl EntityFilter {
    pub fn from_config(cfg: &FilterConfig) -> Self {
        Self {
            include_prefixes: cfg.include_prefixes.clone(),
            exclude_prefixes: cfg.exclude_prefixes.clone(),
            include_exact:    cfg.include_exact.iter().cloned().collect(),
        }
    }

    /// Accept `id` if it is in `include_exact`, OR if it matches an
    /// `include_prefix` AND does not match any `exclude_prefix`.
    pub fn accepts(&self, id: &str) -> bool {
        if self.include_exact.contains(id) { return true; }
        let inc = self.include_prefixes.iter().any(|p| id.starts_with(p.as_str()));
        if !inc { return false; }
        !self.exclude_prefixes.iter().any(|p| id.starts_with(p.as_str()))
    }
}

// ── Catalog loading ──────────────────────────────────────────────────────────

/// Parse a TOML string into a `Catalog`.
/// Called from main.rs with the string compiled in via `include_str!`.
pub fn parse_catalog(toml_str: &str) -> Result<Catalog> {
    let catalog: Catalog = toml::from_str(toml_str)?;
    Ok(catalog)
}

// ── Normalization recipe lookup ───────────────────────────────────────────────

/// Resolve a normalization recipe by name.
/// First checks the catalog's `[normalize.*]` section (stateless, from TOML).
/// Then checks the hardcoded stateful recipe table.
fn resolve_recipe(name: &str, catalog: &Catalog) -> Result<NormalizeRecipe> {
    if let Some(r) = catalog.normalize.get(name) {
        return Ok(r.clone());
    }
    match name {
        "zscore_window_20"   => Ok(NormalizeRecipe::ZscoreWindow20),
        "ema_relative_third" => Ok(NormalizeRecipe::EmaRelativeThird),
        _ => Err(anyhow::anyhow!("unknown normalize recipe '{}'", name)),
    }
}

// ── Poller builder ───────────────────────────────────────────────────────────

/// Build one `ShapePoller` per feed in the catalog.
///
/// For each feed signal, calls `registry.register()` to get its ID (all
/// catalog signals must be pre-seeded — a fresh allocation is a bug and
/// causes a startup panic), then registers the signal on `bus`.
///
/// Emits one startup log line per feed:
/// `INFO feed=flux-weather shape=flat_multi entities=6 signals=18`
pub fn build_pollers(
    catalog:  &Catalog,
    registry: &mut SignalRegistry,
    state:    Arc<Mutex<MultiFluxState>>,
    bus:      &mut SignalBus,
) -> Result<Vec<Box<dyn ShapePoller>>> {
    let mut pollers: Vec<Box<dyn ShapePoller>> = Vec::new();

    for feed in &catalog.feed {
        match feed.shape.as_str() {

            // ── flat_multi ──────────────────────────────────────────────────
            "flat_multi" => {
                let mut entries: Vec<(String, String, SignalId, NormalizeRecipe)> = Vec::new();
                let mut seeded_count: u32 = 0;
                let mut fresh_count:  u32 = 0;

                for entity_name in &feed.entities {
                    for prop in &feed.property {
                        // Use `name` as the registry key suffix if present; fall back to
                        // `path`.  This decouples the stable signal ID from the extraction
                        // path, so a Flux schema change (path rename) can be absorbed in the
                        // catalog without orphaning existing seeded IDs.
                        let key_suffix = prop.name.as_deref().unwrap_or(&prop.path);
                        // Snapshot HWM *before* register() — fresh iff returned id > pre_hwm.
                        let pre_hwm = registry.high_water_mark();
                        let id = registry.register(&feed.namespace, entity_name, key_suffix)?;
                        if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }
                        let recipe = resolve_recipe(&prop.normalize, catalog)?;
                        let decay = decay_for_namespace(&feed.namespace);
                        bus.register_with_id(id, SignalClass::World, 0.5, decay, 0.0);
                        // prop.path is the JSON extraction path (unchanged); key_suffix is
                        // only the registry key.  Both are decoupled when `name` is set.
                        entries.push((entity_name.clone(), prop.path.clone(), id, recipe));
                    }
                }

                let n_entities = feed.entities.len();
                let poller = flat_multi::FlatMultiPoller::new(state.clone(), feed.namespace.clone(), entries);
                pollers.push(Box::new(poller));
                let p = pollers.last().unwrap();
                tracing::info!(
                    "feed={} shape={} entities={} signals={} seeded={} fresh={}",
                    feed.namespace, p.shape_name(), n_entities, p.signal_count(),
                    seeded_count, fresh_count
                );
            }

            // ── scalar ──────────────────────────────────────────────────────
            "scalar" => {
                let mut entries: Vec<(String, String, SignalId, NormalizeRecipe, bool)> = Vec::new();
                let mut seeded_count: u32 = 0;
                let mut fresh_count:  u32 = 0;

                for entity_cfg in &feed.entity_list {
                    let entity_path = entity_cfg.path.as_deref()
                        .ok_or_else(|| anyhow::anyhow!(
                            "scalar entity '{}' in '{}' missing 'path'",
                            entity_cfg.name, feed.namespace
                        ))?;
                    let entity_norm = entity_cfg.normalize.as_deref()
                        .ok_or_else(|| anyhow::anyhow!(
                            "scalar entity '{}' in '{}' missing 'normalize'",
                            entity_cfg.name, feed.namespace
                        ))?;
                    let pre_hwm = registry.high_water_mark();
                    let id = registry.register(&feed.namespace, &entity_cfg.name, entity_path)?;
                    if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }
                    let recipe = resolve_recipe(entity_norm, catalog)?;
                    let decay = decay_for_namespace(&feed.namespace);
                    bus.register_with_id(id, SignalClass::World, 0.5, decay, 0.0);
                    let entity_id = format!("{}/{}", feed.namespace, entity_cfg.name);
                    entries.push((entity_id, entity_path.to_string(), id, recipe, entity_cfg.parse_string));
                }

                let n = entries.len();
                let poller = scalar::ScalarPoller::new(state.clone(), entries);
                pollers.push(Box::new(poller));
                let p = pollers.last().unwrap();
                tracing::info!(
                    "feed={} shape={} entities={} signals={} seeded={} fresh={}",
                    feed.namespace, p.shape_name(), n, p.signal_count(),
                    seeded_count, fresh_count
                );
            }

            // ── nested_by_key ───────────────────────────────────────────────
            "nested_by_key" => {
                use crate::shape::nested_by_key::{NestedByKeyPoller, SignalExtract};

                let n_entities = feed.entity_list.len();
                let n_signals  = feed.signal_list.len();
                let mut flat_entries = Vec::with_capacity(n_entities * n_signals);
                let mut seeded_count: u32 = 0;
                let mut fresh_count:  u32 = 0;

                for entity_def in &feed.entity_list {
                    let entity_key = entity_def.key.as_deref()
                        .ok_or_else(|| anyhow::anyhow!(
                            "nested_by_key entity '{}' in '{}' missing 'key'",
                            entity_def.name, feed.namespace
                        ))?;

                    for sig_def in &feed.signal_list {
                        let pre_hwm = registry.high_water_mark();
                        let id = registry.register(&feed.namespace, &entity_def.name, &sig_def.name)?;
                        if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }

                        let recipe = resolve_recipe(&sig_def.normalize, catalog)?;
                        let decay = decay_for_namespace(&feed.namespace);
                        bus.register_with_id(id, SignalClass::World, 0.5, decay, 0.0);

                        let extract = if let Some(derive_name) = &sig_def.derive {
                            let derive_recipe = derive::DeriveRecipe::from_name(derive_name)
                                .ok_or_else(|| anyhow::anyhow!(
                                    "unknown derive recipe '{}' in signal '{}' of '{}'",
                                    derive_name, sig_def.name, feed.namespace
                                ))?;
                            let from_path = sig_def.from_path.clone()
                                .ok_or_else(|| anyhow::anyhow!(
                                    "derived signal '{}' in '{}' missing 'from_path'",
                                    sig_def.name, feed.namespace
                                ))?;
                            let ref_path = sig_def.ref_path.clone()
                                .ok_or_else(|| anyhow::anyhow!(
                                    "derived signal '{}' in '{}' missing 'ref_path'",
                                    sig_def.name, feed.namespace
                                ))?;
                            SignalExtract::Derived {
                                recipe: derive_recipe,
                                from_path,
                                ref_path,
                                parse_string: sig_def.parse_string,
                            }
                        } else {
                            let path = sig_def.path.clone()
                                .ok_or_else(|| anyhow::anyhow!(
                                    "signal '{}' in '{}' missing 'path'",
                                    sig_def.name, feed.namespace
                                ))?;
                            SignalExtract::Path {
                                path,
                                parse_string: sig_def.parse_string,
                                positive: sig_def.positive,
                            }
                        };

                        flat_entries.push((
                            entity_def.name.clone(),
                            entity_key.to_string(),
                            extract,
                            id,
                            recipe,
                        ));
                    }
                }

                let poller = NestedByKeyPoller::new(
                    state.clone(), feed.namespace.clone(),
                    n_entities, n_signals, flat_entries,
                );
                pollers.push(Box::new(poller));
                let p = pollers.last().unwrap();
                tracing::info!(
                    "feed={} shape={} entities={} signals={} seeded={} fresh={}",
                    feed.namespace, p.shape_name(), n_entities, p.signal_count(),
                    seeded_count, fresh_count
                );
            }

            // ── array_record ────────────────────────────────────────────────
            "array_record" => {
                use crate::shape::array_record::ArrayRecordPoller;

                let n_entities = feed.entities.len();
                let n_props    = feed.property.len();
                let mut flat_entries = Vec::with_capacity(n_entities * n_props);
                let mut seeded_count: u32 = 0;
                let mut fresh_count:  u32 = 0;

                for entity_name in &feed.entities {
                    for prop in &feed.property {
                        let prop_name = prop.name.as_deref()
                            .ok_or_else(|| anyhow::anyhow!(
                                "array_record property missing 'name' in feed '{}'",
                                feed.namespace
                            ))?;
                        let pre_hwm = registry.high_water_mark();
                        let id = registry.register(&feed.namespace, entity_name, prop_name)?;
                        if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }

                        let recipe = resolve_recipe(&prop.normalize, catalog)?;
                        let decay = decay_for_namespace(&feed.namespace);
                        bus.register_with_id(id, SignalClass::World, 0.5, decay, 0.0);

                        flat_entries.push((
                            entity_name.clone(),
                            prop.path.clone(),
                            prop.parse_string,
                            prop.positive,
                            id,
                            recipe,
                        ));
                    }
                }

                let poller = ArrayRecordPoller::new(
                    state.clone(), feed.namespace.clone(),
                    n_entities, n_props, flat_entries,
                );
                pollers.push(Box::new(poller));
                let p = pollers.last().unwrap();
                tracing::info!(
                    "feed={} shape={} entities={} signals={} seeded={} fresh={}",
                    feed.namespace, p.shape_name(), n_entities, p.signal_count(),
                    seeded_count, fresh_count
                );
            }

            // ── time_series ─────────────────────────────────────────────────
            "time_series" => {
                use crate::shape::time_series::TimeSeriesPoller;

                let signal_name = feed.signal_name.as_deref()
                    .ok_or_else(|| anyhow::anyhow!(
                        "time_series feed '{}' missing 'signal_name'", feed.namespace
                    ))?;
                let path = feed.path.as_deref()
                    .ok_or_else(|| anyhow::anyhow!(
                        "time_series feed '{}' missing 'path'", feed.namespace
                    ))?;
                let normalize_name = feed.normalize.as_deref()
                    .ok_or_else(|| anyhow::anyhow!(
                        "time_series feed '{}' missing 'normalize'", feed.namespace
                    ))?;

                let mut flat_entries = Vec::with_capacity(feed.entities.len());
                let mut seeded_count: u32 = 0;
                let mut fresh_count:  u32 = 0;

                for entity_name in &feed.entities {
                    let pre_hwm = registry.high_water_mark();
                    let id = registry.register(&feed.namespace, entity_name, signal_name)?;
                    if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }

                    let recipe = resolve_recipe(normalize_name, catalog)?;
                    let decay = decay_for_namespace(&feed.namespace);
                    bus.register_with_id(id, SignalClass::World, 0.5, decay, 0.0);

                    flat_entries.push((
                        entity_name.clone(),
                        path.to_string(),
                        feed.parse_string,
                        feed.positive,
                        id,
                        recipe,
                    ));
                }

                let n = flat_entries.len();
                let poller = TimeSeriesPoller::new(state.clone(), feed.namespace.clone(), flat_entries);
                pollers.push(Box::new(poller));
                let p = pollers.last().unwrap();
                tracing::info!(
                    "feed={} shape={} entities={} signals={} seeded={} fresh={}",
                    feed.namespace, p.shape_name(), n, p.signal_count(),
                    seeded_count, fresh_count
                );
            }

            // ── aggregation ─────────────────────────────────────────────────
            "aggregation" => {
                use crate::shape::aggregation::{AggregationPoller, AggregateSpec};

                let mut bindings = Vec::with_capacity(feed.aggregate.len());
                let mut seeded_count: u32 = 0;
                let mut fresh_count:  u32 = 0;

                for agg_cfg in &feed.aggregate {
                    let agg_name = format!("agg.{}", agg_cfg.name);
                    // Key format: `{namespace}#agg.{name}` (no entity component)
                    let pre_hwm = registry.high_water_mark();
                    let id = registry.register(&feed.namespace, "", &agg_name)?;
                    if id.0 > pre_hwm { fresh_count += 1; } else { seeded_count += 1; }

                    let recipe = match &agg_cfg.normalize {
                        NormalizeRef::Named(name) => resolve_recipe(name, catalog)?,
                        NormalizeRef::Inline(r)   => r.clone(),
                    };
                    let decay = decay_for_namespace(&feed.namespace);
                    bus.register_with_id(id, SignalClass::World, 0.5, decay, 0.0);

                    let spec = AggregateSpec::from_name(&agg_cfg.name)
                        .map_err(|e| anyhow::anyhow!(
                            "aggregation feed '{}' aggregate '{}': {}",
                            feed.namespace, agg_cfg.name, e
                        ))?;

                    bindings.push((spec, id, recipe));
                }

                let n_signals = bindings.len();
                let poller = AggregationPoller::new(
                    state.clone(), feed.namespace.clone(), bindings,
                );
                pollers.push(Box::new(poller));
                tracing::info!(
                    "feed={} shape=aggregation entities=* signals={} seeded={} fresh={}",
                    feed.namespace, n_signals, seeded_count, fresh_count
                );
            }

            other => {
                tracing::warn!(
                    "unknown feed shape '{}' in namespace '{}' — skipped",
                    other, feed.namespace
                );
            }
        }
    }

    Ok(pollers)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Decay rate for bus signal registration, keyed by namespace prefix.
fn decay_for_namespace(namespace: &str) -> f64 {
    if namespace.starts_with("flux-economic") {
        0.0002 // monthly/quarterly updates
    } else if namespace.starts_with("flux-internet") {
        0.002  // hourly updates
    } else if namespace.starts_with("flux-stocks") {
        0.001  // prev-day OHLCV, once per day
    } else if namespace.starts_with("flux-commodities") {
        0.001  // daily/monthly commodity prices
    } else if namespace.starts_with("flux-crypto") {
        0.005  // Kraken feed, updates every few minutes
    } else if namespace.starts_with("flux-aviation") {
        0.002  // entity counts fluctuate minute-to-minute
    } else if namespace.starts_with("flux-ships") {
        0.001  // vessel counts change slowly
    } else {
        0.001  // default
    }
}
