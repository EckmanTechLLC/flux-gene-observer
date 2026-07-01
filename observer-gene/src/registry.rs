//! Persistent signal-ID registry for observer-gene.
//!
//! This registry covers seed-time and catalog-driven signals (IDs 0–189 and future
//! catalog allocations above 189). Runtime-allocated derived signals
//! (CoinDerivedSignal, IDs 200+) are **not** managed here — they continue to use
//! the existing runtime allocation path directly on the ActionSpace.
//!
//! ## Key format
//! `{namespace}/{entity}#{property_path}`
//!
//! For aggregate signals with no specific entity (aviation, ships, earthquakes):
//! `{namespace}#{property_path}`
//!
//! For internal signals (continuity, meta, drive):
//! `internal/{name}#value`
//!
//! Examples:
//! ```text
//! internal/continuity#value                               → 0
//! flux-weather/new-york#current.temperature_2m            → 10
//! flux-crypto/bitcoin#price                               → 30
//! flux-stocks/SPY#close                                   → 50
//! flux-aviation-europe#agg.count                          → 70
//! flux-ships-north-sea#agg.count                          → 90
//! flux-earthquakes#agg.rate                               → 100
//! flux-commodities/brent-crude#derived.zscore             → 130
//! flux-economic/us-consumer-sentiment#value               → 140
//! flux-internet/global-traffic#result.summary_0.mobile    → 150
//! ```
//!
//! ## Persistence
//! Runtime file: `observer-data/signal-registry.json`.
//! Writes are atomic: data goes to a `.tmp` sibling, then `rename` swaps it in.
//! A crash during write loses at most one allocation; the file is always valid JSON.
//!
//! ## Seeding
//! On first run (runtime file absent), the seed JSON string is written verbatim to
//! the runtime path. `main.rs` compiles the seed into the binary via `include_str!`
//! so deployments are self-contained. After the first run the runtime file is
//! canonical; the seed is never consulted again.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gene_core::signal::types::SignalId;
use serde::{Deserialize, Serialize};

// ── Serialized form (direct JSON mapping) ────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct RegistryData {
    high_water_mark: u32,
    by_key:          BTreeMap<String, u32>,
    reserved:        Vec<String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Persistent mapping from `{namespace}/{entity}#{property}` keys to `SignalId`s.
// dead_code allowed: register/retire/iter/get are pub API for tasks 07-10;
// path and persist are used by those methods. All methods will be called once
// catalog-driven pollers replace the hand-coded ones.
#[allow(dead_code)]
pub struct SignalRegistry {
    data: RegistryData,
    path: PathBuf,
}

#[allow(dead_code)]
impl SignalRegistry {
    // ── Construction ─────────────────────────────────────────────────────────

    /// Load the registry from `path`. If the file does not exist, write `seed_json`
    /// to it first. If neither exists (seed_json is empty), returns an error and
    /// refuses to start.
    pub fn load_or_seed(path: &Path, seed_json: &str) -> Result<Self> {
        if !path.exists() {
            if seed_json.is_empty() {
                anyhow::bail!(
                    "signal registry not found at {:?} and no seed provided; cannot start",
                    path
                );
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating registry directory {:?}", parent))?;
            }
            std::fs::write(path, seed_json)
                .with_context(|| format!("writing seed registry to {:?}", path))?;
            tracing::info!("signal registry: seeded → {:?}", path);
        }

        let json = std::fs::read_to_string(path)
            .with_context(|| format!("reading registry {:?}", path))?;
        let data: RegistryData = serde_json::from_str(&json)
            .with_context(|| format!("parsing registry {:?}", path))?;

        tracing::info!(
            "signal registry: loaded {} signals, high_water_mark={}, reserved={}",
            data.by_key.len(),
            data.high_water_mark,
            data.reserved.len()
        );

        Ok(SignalRegistry { data, path: path.to_owned() })
    }

    // ── Key construction ──────────────────────────────────────────────────────

    fn make_key(namespace: &str, entity: &str, property: &str) -> String {
        if entity.is_empty() {
            format!("{}#{}", namespace, property)
        } else {
            format!("{}/{}#{}", namespace, entity, property)
        }
    }

    // ── Lookup ────────────────────────────────────────────────────────────────

    /// Look up a signal ID by (namespace, entity, property).
    /// Returns `None` if not registered; does not allocate.
    pub fn get(&self, namespace: &str, entity: &str, property: &str) -> Option<SignalId> {
        let key = Self::make_key(namespace, entity, property);
        self.data.by_key.get(&key).copied().map(SignalId)
    }

    /// Look up a signal ID by its full key string
    /// (e.g. `"flux-weather/new-york#current.temperature_2m"`).
    /// Returns `None` if not registered; does not allocate.
    pub fn get_by_key(&self, key: &str) -> Option<SignalId> {
        self.data.by_key.get(key).copied().map(SignalId)
    }

    // ── Allocation ────────────────────────────────────────────────────────────

    /// Return the existing `SignalId` for the key, or allocate a new one above the
    /// current `high_water_mark`. Persists immediately on allocation.
    ///
    /// Returns `Err` only on disk failure.
    pub fn register(&mut self, namespace: &str, entity: &str, property: &str) -> Result<SignalId> {
        let key = Self::make_key(namespace, entity, property);
        if let Some(&id) = self.data.by_key.get(&key) {
            return Ok(SignalId(id));
        }
        self.data.high_water_mark += 1;
        let id = self.data.high_water_mark;
        self.data.by_key.insert(key, id);
        self.persist()?;
        Ok(SignalId(id))
    }

    // ── Retirement ────────────────────────────────────────────────────────────

    /// Move the key from `by_key` into `reserved`. Future `register()` calls for
    /// the same tuple will allocate a fresh ID, leaving the retired ID permanently
    /// un-reusable. Used when retiring a feed whose signal IDs may still appear in
    /// the symbol ledger.
    pub fn retire(&mut self, namespace: &str, entity: &str, property: &str) -> Result<()> {
        let key = Self::make_key(namespace, entity, property);
        if self.data.by_key.remove(&key).is_some() {
            if !self.data.reserved.contains(&key) {
                self.data.reserved.push(key);
            }
            self.persist()?;
        }
        Ok(())
    }

    // ── High-water mark ───────────────────────────────────────────────────────

    /// Returns the current high-water mark (highest allocated signal ID).
    /// Snapshot this *before* calling `register()` to detect whether the
    /// next allocation is fresh: `fresh = new_id.0 > pre_hwm`.
    pub fn high_water_mark(&self) -> u32 {
        self.data.high_water_mark
    }

    // ── Iteration ─────────────────────────────────────────────────────────────

    /// Iterate all registered (key, SignalId) pairs in key-sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, SignalId)> {
        self.data.by_key.iter().map(|(k, &id)| (k.as_str(), SignalId(id)))
    }

    // ── Persistence ───────────────────────────────────────────────────────────

    /// Atomically write the registry to disk.
    /// Writes to `{path}.tmp` then renames over `{path}`.
    fn persist(&self) -> Result<()> {
        // Build tmp path: same dir, same filename with ".tmp" appended
        let mut tmp_name: OsString = self
            .path
            .file_name()
            .unwrap_or_default()
            .to_owned();
        tmp_name.push(".tmp");
        let tmp = self.path.with_file_name(tmp_name);

        let json = serde_json::to_string_pretty(&self.data)
            .context("serializing registry")?;
        std::fs::write(&tmp, &json)
            .with_context(|| format!("writing registry tmp {:?}", tmp))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("renaming registry tmp {:?} → {:?}", tmp, self.path))?;
        Ok(())
    }
}
