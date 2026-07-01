//! FlatMultiPoller — `flat_multi` shape driver.
//!
//! Shape: a fixed list of entities (e.g. city names) and a fixed list of
//! property paths. For each entity, each property is read from
//! `state.entities["{namespace}/{entity}"]`, navigated via the dotted path,
//! normalized, and written to the bus.
//!
//! Example: `flux-weather` with entities=["new-york", ...] and paths like
//! `"current.temperature_2m"` where `current` is the top-level property key
//! and `temperature_2m` is nested inside its JSON value.

use std::sync::{Arc, Mutex};

use gene_core::signal::bus::SignalBus;
use gene_core::signal::types::SignalId;

use crate::shape::normalize::{NormalizeRecipe, NormalizeState};
use crate::shape::path;
use crate::shape::ShapePoller;
use crate::signal::flux_multi::MultiFluxState;

struct SignalEntry {
    entity_name: String,
    prop_path:   String,
    id:          SignalId,
    recipe:      NormalizeRecipe,
    state:       NormalizeState,
}

pub struct FlatMultiPoller {
    state:       Arc<Mutex<MultiFluxState>>,
    namespace:   String,
    initialized: bool,
    signals:     Vec<SignalEntry>,
}

impl FlatMultiPoller {
    /// `signal_entries`: `(entity_name, prop_path, signal_id, recipe)` tuples,
    /// ordered entities × properties (outer entity, inner property).
    pub fn new(
        state:          Arc<Mutex<MultiFluxState>>,
        namespace:      String,
        signal_entries: Vec<(String, String, SignalId, NormalizeRecipe)>,
    ) -> Self {
        let signals = signal_entries
            .into_iter()
            .map(|(entity_name, prop_path, id, recipe)| {
                let state = recipe.init_state();
                SignalEntry { entity_name, prop_path, id, recipe, state }
            })
            .collect();
        Self { state, namespace, initialized: false, signals }
    }
}

impl ShapePoller for FlatMultiPoller {
    fn poll(&mut self, bus: &mut SignalBus) {
        if !self.initialized {
            self.initialized = true;
            return;
        }

        let Ok(flux_state) = self.state.try_lock() else { return };

        // Collect (signal_idx, raw_value) while holding lock (immutable borrow of signals)
        let mut raw_updates: Vec<(usize, f64)> = Vec::with_capacity(self.signals.len());

        for (idx, entry) in self.signals.iter().enumerate() {
            let entity_id = format!("{}/{}", self.namespace, entry.entity_name);
            let Some(props) = flux_state.entities.get(&entity_id) else { continue };

            // First segment = property name in the entity HashMap;
            // remainder = nested path within the property's JSON Value.
            let (prop_key, nested) = path::split_first(&entry.prop_path);
            let Some(top_val) = props.get(prop_key) else { continue };

            let raw: Option<f64> = if nested.is_empty() {
                top_val.as_f64()
            } else {
                // flat_multi properties are always numeric (no parse_string)
                path::resolve_f64(top_val, nested, false)
            };

            if let Some(val) = raw {
                raw_updates.push((idx, val));
            }
        }

        drop(flux_state); // release lock before touching bus

        // Apply normalization (mutates per-signal state) and write to bus
        for (idx, val) in raw_updates {
            let entry = &mut self.signals[idx];
            let norm = entry.recipe.apply(val, &mut entry.state);
            bus.set_value(entry.id, norm);
        }
    }

    fn shape_name(&self) -> &'static str { "flat_multi" }

    fn signal_count(&self) -> usize { self.signals.len() }
}
