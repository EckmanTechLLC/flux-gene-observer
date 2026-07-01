//! Derivation primitives — compute a derived signal value from two extracted values.

/// A recipe for computing a derived signal from a `from` value and a `ref` value.
pub enum DeriveRecipe {
    /// `(from − ref) / ref × 100`. Returns 0 if `ref ≤ 0`.
    PctChange,
}

impl DeriveRecipe {
    pub fn compute(&self, from: f64, ref_val: f64) -> f64 {
        match self {
            Self::PctChange => {
                if ref_val > 0.0 { (from - ref_val) / ref_val * 100.0 }
                else { 0.0 }
            }
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pct_change" => Some(Self::PctChange),
            _            => None,
        }
    }
}
