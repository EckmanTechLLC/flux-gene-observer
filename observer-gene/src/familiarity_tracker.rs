use gene_core::signal::types::SignalId;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

const WINDOW: usize = 500;

/// Tracks recent pattern_ids and computes familiarity = recent-occurrence
/// frequency of the current pattern_id.
pub struct FamiliarityTracker {
    history: VecDeque<u64>,
}

impl FamiliarityTracker {
    pub fn new() -> Self {
        Self { history: VecDeque::with_capacity(WINDOW + 1) }
    }

    /// Compute a stable pattern_id from the current active-signal set.
    /// Order-independent: sorts signal IDs before hashing.
    pub fn pattern_id(active: &HashMap<SignalId, f64>) -> u64 {
        let mut ids: Vec<u32> = active.keys().map(|s| s.0).collect();
        ids.sort_unstable();
        let mut h = DefaultHasher::new();
        ids.hash(&mut h);
        h.finish()
    }

    /// Push the current pattern_id onto the window. Trims to WINDOW.
    pub fn observe(&mut self, pattern_id: u64) {
        self.history.push_back(pattern_id);
        if self.history.len() > WINDOW {
            self.history.pop_front();
        }
    }

    /// Familiarity: count of pattern_id in history / window size.
    /// Returns 0.0 if window is empty.
    pub fn familiarity(&self, pattern_id: u64) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }
        let hits = self.history.iter().filter(|&&id| id == pattern_id).count();
        hits as f64 / self.history.len() as f64
    }
}

impl Default for FamiliarityTracker {
    fn default() -> Self { Self::new() }
}
