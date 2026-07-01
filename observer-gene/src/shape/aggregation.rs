//! Aggregation shape: scan all entities under a namespace prefix and compute
//! count + mean-per-property aggregates.  Used by aviation and ships feeds.
//!
//! Registry key format: `{namespace}#agg.{name}` (no entity component).
//! `registry.register(namespace, "", "agg.count")` → `flux-aviation-europe#agg.count`

use std::sync::{Arc, Mutex};

use gene_core::signal::bus::SignalBus;
use gene_core::signal::types::SignalId;

use crate::shape::normalize::{NormalizeRecipe, NormalizeState};
use crate::shape::ShapePoller;
use crate::signal::flux_multi::MultiFluxState;

// ── Aggregate spec ────────────────────────────────────────────────────────────

/// Which aggregate to compute from the entity set.
pub enum AggregateSpec {
    /// Count of entities whose key starts with the namespace prefix.
    Count,
    /// Arithmetic mean of a flat numeric property across all matched entities.
    Mean { property: String },
    /// Fraction of entities where a property equals a target string value.
    ///
    /// JSON strings compare as-is (`"YELLOW"` → `YELLOW`).
    /// Bools/numbers are stringified via `Value::to_string()` (`false` → `"false"`).
    /// Entities missing the property are counted in the denominator but not the
    /// numerator, so they reduce the fraction.
    Fraction { property: String, value: String },
}

impl AggregateSpec {
    /// Parse the catalog `name` field:
    /// - `"count"`                    → `Count`
    /// - `"mean.speed_ms"`            → `Mean { property: "speed_ms" }`
    /// - `"fraction.color_code=YELLOW"` → `Fraction { property: "color_code", value: "YELLOW" }`
    pub fn from_name(name: &str) -> anyhow::Result<Self> {
        if name == "count" {
            return Ok(Self::Count);
        }
        if let Some(prop) = name.strip_prefix("mean.") {
            if prop.is_empty() {
                anyhow::bail!("aggregate name 'mean.' has empty property part");
            }
            return Ok(Self::Mean { property: prop.to_string() });
        }
        if let Some(rest) = name.strip_prefix("fraction.") {
            if let Some((prop, val)) = rest.split_once('=') {
                if prop.is_empty() {
                    anyhow::bail!("aggregate name '{}' has empty property part", name);
                }
                return Ok(Self::Fraction {
                    property: prop.to_string(),
                    value:    val.to_string(),
                });
            }
            anyhow::bail!(
                "aggregate name '{}' missing '=<value>' part; expected 'fraction.<prop>=<val>'",
                name
            );
        }
        anyhow::bail!(
            "unknown aggregate name '{}'; expected 'count', 'mean.<property>', or 'fraction.<property>=<value>'",
            name
        )
    }
}

// ── Per-aggregate binding ─────────────────────────────────────────────────────

struct AggregateBinding {
    spec:      AggregateSpec,
    signal_id: SignalId,
    recipe:    NormalizeRecipe,
    /// Stateful normalization (None for LinearRange, which is the typical case here).
    state:     NormalizeState,
}

// ── Poller ────────────────────────────────────────────────────────────────────

pub struct AggregationPoller {
    /// `"{namespace}/"` — prefix for entity-ID matching.
    namespace_prefix: String,
    aggregates:       Vec<AggregateBinding>,
    flux_state:       Arc<Mutex<MultiFluxState>>,
    initialized:      bool,
    n_signals:        usize,
}

impl AggregationPoller {
    /// `bindings`: `(spec, signal_id, recipe)` — one per `[[feed.aggregate]]` entry.
    pub fn new(
        flux_state: Arc<Mutex<MultiFluxState>>,
        namespace:  String,
        bindings:   Vec<(AggregateSpec, SignalId, NormalizeRecipe)>,
    ) -> Self {
        let n_signals = bindings.len();
        let namespace_prefix = format!("{}/", namespace);
        let aggregates = bindings
            .into_iter()
            .map(|(spec, signal_id, recipe)| {
                let state = recipe.init_state();
                AggregateBinding { spec, signal_id, recipe, state }
            })
            .collect();
        Self {
            namespace_prefix,
            aggregates,
            flux_state,
            initialized: false,
            n_signals,
        }
    }
}

impl ShapePoller for AggregationPoller {
    fn poll(&mut self, bus: &mut SignalBus) {
        // First call: record that we are initialized and skip emission.
        // (Same pattern as AviationPoller / ShipsPoller / WorldSignalPoller.)
        if !self.initialized {
            self.initialized = true;
            return;
        }

        // try_lock: bail if the WS task is mid-write.
        let Ok(state) = self.flux_state.try_lock() else { return };

        let n = self.aggregates.len();
        let mut entity_count:   u64      = 0;
        let mut sums:           Vec<f64> = vec![0.0; n];
        let mut mean_ns:        Vec<u64> = vec![0;   n];
        // Fraction accumulators: total entities in namespace, hits per aggregate.
        let mut frac_totals:    Vec<u64> = vec![0;   n];
        let mut frac_hits:      Vec<u64> = vec![0;   n];

        for (entity_id, props) in &state.entities {
            if !entity_id.starts_with(&self.namespace_prefix) { continue; }
            entity_count += 1;
            for (i, agg) in self.aggregates.iter().enumerate() {
                match &agg.spec {
                    AggregateSpec::Mean { property } => {
                        if let Some(v) = props.get(property.as_str()).and_then(|v| v.as_f64()) {
                            sums[i]    += v;
                            mean_ns[i] += 1;
                        }
                    }
                    AggregateSpec::Fraction { property, value } => {
                        // Every entity in the namespace counts toward the denominator,
                        // even those where the property is missing.
                        frac_totals[i] += 1;
                        if let Some(prop_val) = props.get(property.as_str()) {
                            // Strings compare as-is; bools/numbers via to_string().
                            // Special-casing String avoids spurious double-quotes from
                            // Value::to_string() (which would produce `"YELLOW"` not `YELLOW`).
                            let val_str = match prop_val {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            if val_str == value.as_str() {
                                frac_hits[i] += 1;
                            }
                        }
                    }
                    AggregateSpec::Count => {} // counted by entity_count
                }
            }
        }

        drop(state); // release lock before touching the bus

        let mut updates: Vec<(SignalId, f64)> = Vec::with_capacity(n);
        for (i, agg) in self.aggregates.iter_mut().enumerate() {
            let raw = match &agg.spec {
                AggregateSpec::Count => entity_count as f64,
                AggregateSpec::Mean { .. } => {
                    if mean_ns[i] > 0 { sums[i] / mean_ns[i] as f64 } else { 0.0 }
                }
                AggregateSpec::Fraction { .. } => {
                    if frac_totals[i] > 0 {
                        frac_hits[i] as f64 / frac_totals[i] as f64
                    } else {
                        0.0
                    }
                }
            };
            let val = agg.recipe.apply(raw, &mut agg.state);
            updates.push((agg.signal_id, val));
        }

        for (id, val) in updates {
            bus.set_value(id, val);
        }
    }

    fn shape_name(&self) -> &'static str { "aggregation" }
    fn signal_count(&self) -> usize { self.n_signals }
}
