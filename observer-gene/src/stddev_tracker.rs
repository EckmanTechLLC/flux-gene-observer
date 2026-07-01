use gene_core::signal::types::SignalId;
use std::collections::{HashMap, VecDeque};

const WINDOW_SIZE:   usize = 500;
const K_FACTOR:      f64   = 1.5;
const MIN_THRESHOLD: f64   = 0.005;
const MAX_THRESHOLD: f64   = 0.10;

/// Tracks a rolling standard deviation window per signal and computes
/// per-signal adaptive deviation thresholds as `k × stddev`, clamped to
/// `[MIN_THRESHOLD, MAX_THRESHOLD]`.
pub struct StddevTracker {
    windows: HashMap<SignalId, VecDeque<f64>>,
}

impl StddevTracker {
    pub fn new() -> Self {
        Self { windows: HashMap::new() }
    }

    /// Push the latest value for a signal.  Trims window to WINDOW_SIZE.
    pub fn observe(&mut self, id: SignalId, value: f64) {
        let w = self.windows.entry(id).or_insert_with(VecDeque::new);
        w.push_back(value);
        if w.len() > WINDOW_SIZE {
            w.pop_front();
        }
    }

    /// Compute current per-signal threshold map.
    /// Signals with fewer than 10 observations get MIN_THRESHOLD as a fallback.
    pub fn thresholds(&self) -> HashMap<SignalId, f64> {
        self.windows.iter().map(|(id, w)| {
            let threshold = if w.len() < 10 {
                MIN_THRESHOLD
            } else {
                let mean = w.iter().sum::<f64>() / w.len() as f64;
                let var  = w.iter()
                    .map(|v| (v - mean).powi(2))
                    .sum::<f64>() / w.len() as f64;
                let stddev = var.sqrt();
                (K_FACTOR * stddev).clamp(MIN_THRESHOLD, MAX_THRESHOLD)
            };
            (*id, threshold)
        }).collect()
    }
}

impl Default for StddevTracker {
    fn default() -> Self { Self::new() }
}
