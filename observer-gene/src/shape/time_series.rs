//! TimeSeriesPoller — `time_series` shape driver.
//!
//! Shape: a fixed list of entity names, all sharing the same path and signal name.
//! The path typically addresses `data[0].value` — the most-recent entry of a
//! time-series array. Each entity maps to exactly one bus signal.
//!
//! Example: flux-commodities, 6 entities × 1 signal = 6 bus entries per poll.

use std::sync::{Arc, Mutex};

use gene_core::signal::bus::SignalBus;
use gene_core::signal::types::SignalId;

use crate::shape::normalize::{NormalizeRecipe, NormalizeState};
use crate::shape::path;
use crate::shape::ShapePoller;
use crate::signal::flux_multi::MultiFluxState;

struct Entry {
    entity_name:  String,
    prop_path:    String,
    parse_string: bool,
    positive:     bool,
    id:           SignalId,
    recipe:       NormalizeRecipe,
    state:        NormalizeState,
}

pub struct TimeSeriesPoller {
    flux_state:  Arc<Mutex<MultiFluxState>>,
    namespace:   String,
    initialized: bool,
    entries:     Vec<Entry>,
}

impl TimeSeriesPoller {
    /// `flat_entries`: one per entity — `(entity_name, path, parse_string, positive, id, recipe)`.
    pub fn new(
        flux_state:  Arc<Mutex<MultiFluxState>>,
        namespace:   String,
        flat_entries: Vec<(String, String, bool, bool, SignalId, NormalizeRecipe)>,
    ) -> Self {
        let entries = flat_entries
            .into_iter()
            .map(|(entity_name, prop_path, parse_string, positive, id, recipe)| {
                let state = recipe.init_state();
                Entry { entity_name, prop_path, parse_string, positive, id, recipe, state }
            })
            .collect();
        Self { flux_state, namespace, initialized: false, entries }
    }
}

impl ShapePoller for TimeSeriesPoller {
    fn poll(&mut self, bus: &mut SignalBus) {
        if !self.initialized { self.initialized = true; return; }

        let Ok(flux_state) = self.flux_state.try_lock() else { return };

        let mut raw_updates: Vec<(usize, f64)> = Vec::with_capacity(self.entries.len());

        for (idx, entry) in self.entries.iter().enumerate() {
            let entity_id = format!("{}/{}", self.namespace, entry.entity_name);
            let Some(props) = flux_state.entities.get(&entity_id) else { continue };

            let (prop_key, nested) = path::split_first(&entry.prop_path);
            let Some(top_val) = props.get(prop_key) else { continue };

            let raw: Option<f64> = if nested.is_empty() {
                if entry.parse_string { top_val.as_str().and_then(|s| s.parse().ok()) }
                else { top_val.as_f64() }
            } else {
                path::resolve_f64(top_val, nested, entry.parse_string)
            };

            if let Some(val) = raw {
                if entry.positive && val <= 0.0 { continue; }
                raw_updates.push((idx, val));
            }
        }

        drop(flux_state);

        for (idx, val) in raw_updates {
            let entry = &mut self.entries[idx];
            let norm = entry.recipe.apply(val, &mut entry.state);
            bus.set_value(entry.id, norm);
        }
    }

    fn shape_name(&self) -> &'static str { "time_series" }

    fn signal_count(&self) -> usize { self.entries.len() }
}
