//! NestedByKeyPoller — `nested_by_key` shape driver.
//!
//! Shape: each entity has a named `key` (e.g. a Kraken pair code). Signal paths
//! contain `{key}` which is substituted per entity before resolution.
//! Supports derived signals (`pct_change`) computed from two resolved paths.
//! Holds per-(entity×signal) normalization state (z-score window, EMA).
//!
//! Example: flux-crypto, 14 entities × 3 signals = 42 bus entries per poll.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gene_core::signal::bus::SignalBus;
use gene_core::signal::types::SignalId;
use serde_json::Value;

use crate::shape::derive::DeriveRecipe;
use crate::shape::normalize::{NormalizeRecipe, NormalizeState};
use crate::shape::path;
use crate::shape::ShapePoller;
use crate::signal::flux_multi::MultiFluxState;

// ── Internal types ────────────────────────────────────────────────────────────

pub enum SignalExtract {
    Path {
        path:         String,
        parse_string: bool,
        positive:     bool,
    },
    Derived {
        recipe:       DeriveRecipe,
        from_path:    String,
        ref_path:     String,
        parse_string: bool,
    },
}

/// One (entity × signal) entry. Each has its own bus ID and rolling state.
struct Entry {
    entity_name: String,
    entity_key:  String,
    extract:     SignalExtract,
    id:          SignalId,
    recipe:      NormalizeRecipe,
    state:       NormalizeState,
}

// ── Poller ────────────────────────────────────────────────────────────────────

pub struct NestedByKeyPoller {
    flux_state:  Arc<Mutex<MultiFluxState>>,
    namespace:   String,
    initialized: bool,
    entries:     Vec<Entry>,
    n_entities:  usize,
    n_signals:   usize,
}

impl NestedByKeyPoller {
    /// `flat_entries`: one per (entity × signal), ordered entity0/sig0, entity0/sig1, …
    pub fn new(
        flux_state: Arc<Mutex<MultiFluxState>>,
        namespace:  String,
        n_entities: usize,
        n_signals:  usize,
        flat_entries: Vec<(String, String, SignalExtract, SignalId, NormalizeRecipe)>,
    ) -> Self {
        let entries = flat_entries
            .into_iter()
            .map(|(entity_name, entity_key, extract, id, recipe)| {
                let state = recipe.init_state();
                Entry { entity_name, entity_key, extract, id, recipe, state }
            })
            .collect();
        Self { flux_state, namespace, initialized: false, entries, n_entities, n_signals }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn resolve_one(props: &HashMap<String, Value>, resolved_path: &str, parse_string: bool) -> Option<f64> {
    let (prop_key, nested) = path::split_first(resolved_path);
    let top_val = props.get(prop_key)?;
    if nested.is_empty() {
        if parse_string { top_val.as_str()?.parse::<f64>().ok() } else { top_val.as_f64() }
    } else {
        path::resolve_f64(top_val, nested, parse_string)
    }
}

fn extract_signal(entry: &Entry, props: &HashMap<String, Value>) -> Option<f64> {
    match &entry.extract {
        SignalExtract::Path { path: p, parse_string, positive } => {
            let resolved = p.replace("{key}", &entry.entity_key);
            let val = resolve_one(props, &resolved, *parse_string)?;
            if *positive && val <= 0.0 { return None; }
            Some(val)
        }
        SignalExtract::Derived { recipe, from_path, ref_path, parse_string } => {
            let r_from = from_path.replace("{key}", &entry.entity_key);
            let r_ref  = ref_path.replace("{key}", &entry.entity_key);
            let from_val = resolve_one(props, &r_from, *parse_string)?;
            // If ref can't be resolved, use from_val → pct_change returns 0
            let ref_val  = resolve_one(props, &r_ref,  *parse_string).unwrap_or(from_val);
            Some(recipe.compute(from_val, ref_val))
        }
    }
}

// ── ShapePoller impl ──────────────────────────────────────────────────────────

impl ShapePoller for NestedByKeyPoller {
    fn poll(&mut self, bus: &mut SignalBus) {
        if !self.initialized { self.initialized = true; return; }

        let Ok(flux_state) = self.flux_state.try_lock() else { return };

        // Phase 1: extract raw values while holding the lock (immutable borrow of entries)
        let mut raw_updates: Vec<(usize, f64)> = Vec::with_capacity(self.entries.len());
        for (idx, entry) in self.entries.iter().enumerate() {
            let entity_id = format!("{}/{}", self.namespace, entry.entity_name);
            let Some(props) = flux_state.entities.get(&entity_id) else { continue };
            if let Some(val) = extract_signal(entry, props) {
                raw_updates.push((idx, val));
            }
        }

        drop(flux_state);

        // Phase 2: apply normalization (mutates state) and write to bus
        for (idx, val) in raw_updates {
            let entry = &mut self.entries[idx];
            let norm = entry.recipe.apply(val, &mut entry.state);
            bus.set_value(entry.id, norm);
        }
    }

    fn shape_name(&self) -> &'static str { "nested_by_key" }

    fn signal_count(&self) -> usize { self.n_entities * self.n_signals }
}
