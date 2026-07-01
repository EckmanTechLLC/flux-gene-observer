use serde::Deserialize;
use std::collections::VecDeque;

/// A named normalization recipe. Referenced by name from feed entries in feeds.toml.
/// Stateless variants (LinearRange) are deserialized from the `[normalize.*]` TOML section.
/// Stateful variants (ZscoreWindow20, EmaRelativeThird) are looked up by name in code.
#[derive(Deserialize, Clone)]
#[serde(tag = "type")]
pub enum NormalizeRecipe {
    #[serde(rename = "linear_range")]
    LinearRange { min: f64, max: f64 },
    /// Rolling z-score over a 20-sample window, mapped to [0, 1].
    /// Named "zscore_window_20" in build_pollers lookup. Not in TOML.
    #[serde(skip_deserializing)]
    ZscoreWindow20,
    /// EMA-relative: val / EMA(val, 20) / 3, clamped [0, 1].
    /// Named "ema_relative_third" in build_pollers lookup. Not in TOML.
    #[serde(skip_deserializing)]
    EmaRelativeThird,
}

/// Per-signal mutable state for stateful normalization recipes.
pub enum NormalizeState {
    None,
    Window(VecDeque<f64>),
    Ema { value: f64, seeded: bool },
}

impl NormalizeRecipe {
    /// Return the initial (empty) state for this recipe.
    pub fn init_state(&self) -> NormalizeState {
        match self {
            Self::LinearRange { .. } => NormalizeState::None,
            Self::ZscoreWindow20     => NormalizeState::Window(VecDeque::with_capacity(21)),
            Self::EmaRelativeThird   => NormalizeState::Ema { value: 0.0, seeded: false },
        }
    }

    /// Apply normalization to `val`, updating `state` in place for stateful recipes.
    /// LinearRange ignores `state`.
    pub fn apply(&self, val: f64, state: &mut NormalizeState) -> f64 {
        match (self, state) {
            (Self::LinearRange { min, max }, _) => {
                ((val - min) / (max - min)).clamp(0.0, 1.0)
            }
            (Self::ZscoreWindow20, NormalizeState::Window(w)) => {
                w.push_back(val);
                if w.len() > 20 { w.pop_front(); }
                z_score_to_unit(w, val)
            }
            (Self::EmaRelativeThird, NormalizeState::Ema { value, seeded }) => {
                if !*seeded { *value = val; *seeded = true; }
                *value = (1.0 / 20.0) * val + (19.0 / 20.0) * *value;
                if *value > 0.0 { (val / *value / 3.0).clamp(0.0, 1.0) } else { 0.5 }
            }
            _ => panic!("normalize state/recipe mismatch"),
        }
    }
}

/// Map a rolling window + current value to a z-score in [0, 1].
/// Requires at least 2 samples; returns 0.5 if window is too small or std dev is zero.
/// Maps z ∈ [−3, +3] linearly to [0, 1], clamped at both ends.
pub fn z_score_to_unit(window: &VecDeque<f64>, val: f64) -> f64 {
    if window.len() < 2 { return 0.5; }
    let n    = window.len() as f64;
    let mean = window.iter().sum::<f64>() / n;
    let var  = window.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let std  = var.sqrt();
    if std < 1e-9 { return 0.5; }
    (((val - mean) / std + 3.0) / 6.0).clamp(0.0, 1.0)
}
