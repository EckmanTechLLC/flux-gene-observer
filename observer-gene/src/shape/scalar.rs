//! ScalarPoller — `scalar` shape driver.
//!
//! Shape: each feed has a list of named entities. For each entity, a single
//! value is read from `state.entities[entity_id]`, navigated via the dotted
//! path (first segment = property key, rest = nested JSON path), optionally
//! parsed from a string, normalized, and written to the bus.
//!
//! Examples:
//!   flux-economic: path="value"  → props["value"].as_f64()
//!   flux-internet: path="result.summary_0.mobile", parse_string=true
//!                 → props["result"]["summary_0"]["mobile"].as_str().parse()

use std::sync::{Arc, Mutex};

use gene_core::signal::bus::SignalBus;
use gene_core::signal::types::SignalId;

use crate::shape::normalize::{NormalizeRecipe, NormalizeState};
use crate::shape::path;
use crate::shape::ShapePoller;
use crate::signal::flux_multi::MultiFluxState;

struct SignalEntry {
    entity_id:    String,
    prop_path:    String,
    id:           SignalId,
    recipe:       NormalizeRecipe,
    state:        NormalizeState,
    parse_string: bool,
}

pub struct ScalarPoller {
    state:       Arc<Mutex<MultiFluxState>>,
    initialized: bool,
    signals:     Vec<SignalEntry>,
}

impl ScalarPoller {
    /// `signal_entries`: `(entity_id, prop_path, id, recipe, parse_string)`.
    /// `entity_id` is the full entity ID (e.g. `"flux-economic/us-consumer-sentiment"`).
    pub fn new(
        state:          Arc<Mutex<MultiFluxState>>,
        signal_entries: Vec<(String, String, SignalId, NormalizeRecipe, bool)>,
    ) -> Self {
        let signals = signal_entries
            .into_iter()
            .map(|(entity_id, prop_path, id, recipe, parse_string)| {
                let state = recipe.init_state();
                SignalEntry { entity_id, prop_path, id, recipe, state, parse_string }
            })
            .collect();
        Self { state, initialized: false, signals }
    }
}

impl ShapePoller for ScalarPoller {
    fn poll(&mut self, bus: &mut SignalBus) {
        if !self.initialized {
            self.initialized = true;
            return;
        }

        let Ok(flux_state) = self.state.try_lock() else { return };

        let mut raw_updates: Vec<(usize, f64)> = Vec::with_capacity(self.signals.len());

        for (idx, entry) in self.signals.iter().enumerate() {
            let Some(props) = flux_state.entities.get(&entry.entity_id) else { continue };

            let (prop_key, nested) = path::split_first(&entry.prop_path);
            let Some(top_val) = props.get(prop_key) else { continue };

            let raw: Option<f64> = if nested.is_empty() {
                if entry.parse_string {
                    top_val.as_str().and_then(|s| s.parse::<f64>().ok())
                } else {
                    top_val.as_f64()
                }
            } else {
                path::resolve_f64(top_val, nested, entry.parse_string)
            };

            if let Some(val) = raw {
                raw_updates.push((idx, val));
            }
        }

        drop(flux_state); // release lock before touching bus

        for (idx, val) in raw_updates {
            let entry = &mut self.signals[idx];
            let norm = entry.recipe.apply(val, &mut entry.state);
            bus.set_value(entry.id, norm);
        }
    }

    fn shape_name(&self) -> &'static str { "scalar" }

    fn signal_count(&self) -> usize { self.signals.len() }
}
