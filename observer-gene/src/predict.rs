use gene_core::signal::types::SignalId;
use std::collections::{HashMap, VecDeque};

const HORIZON_TICKS: u64   = 5000;
const EMA_ALPHA:     f64   = 0.05;
const TREND_ALPHA:   f64   = 0.01;
const ERROR_WINDOW:  usize = 100;
const NORM_SCALE:    f64   = 0.1;

struct Predictor {
    ema_value:   f64,
    ema_trend:   f64,
    initialized: bool,
    pending:     Option<(u64, f64)>, // (target_tick, predicted_value)
}

impl Predictor {
    fn new() -> Self {
        Self {
            ema_value:   0.0,
            ema_trend:   0.0,
            initialized: false,
            pending:     None,
        }
    }

    fn observe(&mut self, value: f64) {
        if !self.initialized {
            self.ema_value   = value;
            self.ema_trend   = 0.0;
            self.initialized = true;
            return;
        }
        let observed_trend = value - self.ema_value;
        self.ema_trend = TREND_ALPHA * observed_trend + (1.0 - TREND_ALPHA) * self.ema_trend;
        self.ema_value = EMA_ALPHA * value + (1.0 - EMA_ALPHA) * self.ema_value;
    }

    fn predict_at(&self, horizon: u64) -> f64 {
        self.ema_value + (horizon as f64) * self.ema_trend
    }
}

pub struct PredictionEngine {
    predictors: HashMap<SignalId, Predictor>,
    errors:     HashMap<SignalId, VecDeque<f64>>,
}

impl PredictionEngine {
    pub fn new() -> Self {
        Self {
            predictors: HashMap::new(),
            errors:     HashMap::new(),
        }
    }

    /// Observe current values for all signals; resolve any matured predictions;
    /// emit new ones.
    pub fn tick(&mut self, current_tick: u64, snapshot: &[(SignalId, f64)]) {
        for &(id, value) in snapshot {
            let p = self.predictors.entry(id).or_insert_with(Predictor::new);
            p.observe(value);

            // Resolve pending if matured (or stale-evict if overdue without scoring)
            if let Some((target, predicted)) = p.pending {
                if target <= current_tick {
                    let err = (value - predicted).powi(2);
                    let w = self.errors.entry(id).or_insert_with(VecDeque::new);
                    w.push_back(err);
                    if w.len() > ERROR_WINDOW {
                        w.pop_front();
                    }
                    p.pending = None;
                }
            }

            // Emit new prediction if none pending and predictor is initialized
            if p.pending.is_none() && p.initialized {
                let predicted = p.predict_at(HORIZON_TICKS);
                p.pending = Some((current_tick + HORIZON_TICKS, predicted));
            }
        }
    }

    /// Aggregate normalized prediction error across all signals.
    /// Returns 0.0 until the first predictions mature (~5000 ticks).
    /// Steady-state in (0, 1) exclusive.
    pub fn aggregate_error(&self) -> f64 {
        if self.errors.is_empty() {
            return 0.0;
        }
        let mut total = 0.0;
        let mut count = 0usize;
        for window in self.errors.values() {
            if window.is_empty() {
                continue;
            }
            let avg_sq_err = window.iter().sum::<f64>() / window.len() as f64;
            // tanh maps unbounded squared error to [0, 1]
            total += (avg_sq_err / NORM_SCALE).tanh();
            count += 1;
        }
        if count == 0 {
            0.0
        } else {
            total / count as f64
        }
    }
}

impl Default for PredictionEngine {
    fn default() -> Self {
        Self::new()
    }
}
