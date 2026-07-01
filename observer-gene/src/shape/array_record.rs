//! ArrayRecordPoller — `array_record` shape driver.
//!
//! Shape: a fixed list of entity names and a fixed list of named properties.
//! Paths support array indexing (e.g. `"results[0].c"`).
//! Each (entity × property) pair maps to its own bus ID and normalization state.
//!
//! Example: flux-stocks, 5 tickers × 2 properties = 10 bus entries per poll.

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

pub struct ArrayRecordPoller {
    flux_state:  Arc<Mutex<MultiFluxState>>,
    namespace:   String,
    initialized: bool,
    entries:     Vec<Entry>,
    n_entities:  usize,
    n_props:     usize,
}

impl ArrayRecordPoller {
    /// `flat_entries`: one per (entity × property), ordered entity0/prop0, entity0/prop1, …
    pub fn new(
        flux_state:  Arc<Mutex<MultiFluxState>>,
        namespace:   String,
        n_entities:  usize,
        n_props:     usize,
        flat_entries: Vec<(String, String, bool, bool, SignalId, NormalizeRecipe)>,
    ) -> Self {
        let entries = flat_entries
            .into_iter()
            .map(|(entity_name, prop_path, parse_string, positive, id, recipe)| {
                let state = recipe.init_state();
                Entry { entity_name, prop_path, parse_string, positive, id, recipe, state }
            })
            .collect();
        Self { flux_state, namespace, initialized: false, entries, n_entities, n_props }
    }
}

impl ShapePoller for ArrayRecordPoller {
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

    fn shape_name(&self) -> &'static str { "array_record" }

    fn signal_count(&self) -> usize { self.n_entities * self.n_props }
}
