use std::collections::{HashMap, VecDeque};

const WINDOW_TICKS: usize = 1000;

/// Tracks recent symbol-activation distributions and computes a
/// degeneracy score (1.0 = single dominant symbol, 0.0 = perfectly
/// even distribution across all observed symbols).
pub struct DegeneracyTracker {
    /// Per-tick history of active symbol indices.
    history: VecDeque<Vec<u32>>,
}

impl DegeneracyTracker {
    pub fn new() -> Self {
        Self { history: VecDeque::with_capacity(WINDOW_TICKS + 1) }
    }

    /// Record this tick's set of active symbol indices.
    pub fn observe(&mut self, active_indices: Vec<u32>) {
        self.history.push_back(active_indices);
        if self.history.len() > WINDOW_TICKS {
            self.history.pop_front();
        }
    }

    /// Compute degeneracy.
    ///   - 1.0 when only one symbol fires (or no symbols at all)
    ///   - 0.0 when all observed symbols fire equally often
    /// Formula:
    ///   counts[i] = total appearances of symbol i in window
    ///   p_i = counts[i] / total_appearances
    ///   H = -Σ p_i × ln(p_i)
    ///   degeneracy = 1 - (H / ln(N_active))   clamped to [0, 1]
    pub fn degeneracy(&self) -> f64 {
        let mut counts: HashMap<u32, u64> = HashMap::new();
        for tick_active in &self.history {
            for sym_idx in tick_active {
                *counts.entry(*sym_idx).or_insert(0) += 1;
            }
        }
        let n_active = counts.len();
        if n_active <= 1 {
            return 1.0; // single symbol or none → maximally degenerate
        }
        let total: u64 = counts.values().sum();
        if total == 0 {
            return 0.0;
        }
        let entropy: f64 = counts.values()
            .map(|&c| {
                let p = c as f64 / total as f64;
                -p * p.ln()
            })
            .sum();
        let max_entropy = (n_active as f64).ln();
        let h_norm = entropy / max_entropy;
        (1.0 - h_norm).clamp(0.0, 1.0)
    }
}

impl Default for DegeneracyTracker {
    fn default() -> Self { Self::new() }
}
