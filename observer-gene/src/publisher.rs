/// FluxPublisher: publishes observer-gene state and key entities to Flux.
///
/// Two modes:
///   publish_key() — called once at startup; posts observer-gene/key (human-readable
///                   translation of all signal IDs, action IDs, and state fields)
///   publish()     — called every 10,000 ticks; posts observer-gene/state expression record
///
/// Both use HTTP POST /api/events. The background thread handles state publishes
/// (non-blocking, bounded channel). publish_key() posts directly (blocking, once).

use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::registry::SignalRegistry;

pub struct FluxPublisher {
    tx:        mpsc::SyncSender<(String, serde_json::Value)>,
    http_base: String,
    token:     Option<String>,
}

impl FluxPublisher {
    pub fn new(http_base: String, token: Option<String>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<(String, serde_json::Value)>(32);

        let thread_base  = http_base.clone();
        let thread_token = token.clone();

        thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            let url = format!("{}/api/events", thread_base);

            for (entity_id, properties) in rx {
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let body = serde_json::json!({
                    "stream": "observer.gene",
                    "source": "observer-gene",
                    "timestamp": ts,
                    "payload": {
                        "entity_id": entity_id,
                        "properties": properties,
                    }
                });

                let mut req = client.post(&url).json(&body);
                if let Some(ref tok) = thread_token {
                    req = req.bearer_auth(tok);
                }
                if let Err(e) = req.send() {
                    tracing::warn!("flux publish failed: {}", e);
                }
            }
        });

        Self { tx, http_base, token }
    }

    /// Non-blocking state publish. Drops the record silently if the channel is full.
    pub fn publish(&self, record: serde_json::Value) {
        let _ = self.tx.try_send(("observer-gene/state".to_string(), record));
    }

    /// Non-blocking symbol composition publish. Drops silently if the channel is full.
    pub fn publish_symbols(&self, symbol_map: serde_json::Value) {
        let _ = self.tx.try_send(("observer-gene/symbols".to_string(), symbol_map));
    }

    /// Non-blocking key refresh. Drops silently if the channel is full.
    pub fn publish_key_async(&self, registry: &SignalRegistry) {
        let _ = self.tx.try_send(("observer-gene/key".to_string(), key_properties(registry)));
    }

    /// Post the static key entity to observer-gene/key.
    /// Call once at startup. Blocking — completes before the tick loop begins.
    pub fn publish_key(&self, registry: &SignalRegistry) {
        let client = reqwest::blocking::Client::new();
        let url    = format!("{}/api/events", self.http_base);
        let ts     = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let properties = key_properties(registry);

        let body = serde_json::json!({
            "stream": "observer.gene",
            "source": "observer-gene",
            "timestamp": ts,
            "payload": {
                "entity_id": "observer-gene/key",
                "properties": properties,
            }
        });

        let mut req = client.post(&url).json(&body);
        if let Some(ref tok) = self.token {
            req = req.bearer_auth(tok);
        }
        match req.send() {
            Ok(_)  => tracing::info!("observer-gene/key published to flux"),
            Err(e) => tracing::warn!("flux key publish failed: {}", e),
        }
    }
}

// ── Key builders ──────────────────────────────────────────────────────────────

fn key_properties(registry: &SignalRegistry) -> serde_json::Value {
    serde_json::json!({
        "signals": signal_key(registry),
        "actions": action_key(),
        "symbols": "Φ_NNNN: emergent cross-domain co-activation patterns coined at runtime \
                    from correlated world signals. Φ_C_NNNN: higher-order composite symbols \
                    formed from recurring Φ_NNNN combinations. Symbols carry no pre-assigned \
                    meaning — significance emerges from co-activation history.",
        "state_fields": {
            "tick":                  "current tick count since genesis",
            "dominant":              "highest-activation symbol token this tick (Φ_NNNN format)",
            "cluster":               "all symbol tokens active above threshold this tick",
            "imbalance":             "total weighted signal deviation from baseline — lower means more stable",
            "imbalance_trend":       "imbalance direction over last 20 samples: stable / rising / falling",
            "action_context":        "last action selected by the regulation engine",
            "signal_drivers":        "signals with |deviation| > 0.1 from baseline; signed value shows direction and magnitude",
            "self_model_confidence": "self-model prediction accuracy 0–1 (1 = perfect)",
            "identity_alignment":    "fraction of identity-defining symbols currently active 0–1"
        }
    })
}

fn signal_key(registry: &SignalRegistry) -> serde_json::Value {
    let entries: &[(&str, &str)] = &[
        // Continuity & internal
        ("s_0000", "continuity"),
        ("s_0001", "integrity"),
        ("s_0002", "coherence"),
        ("s_0003", "meta.prediction_confidence"),
        ("s_0004", "drive.regulation_urgency"),
        // Weather — 6 cities × 3 properties
        ("s_0010", "weather.temp.new_york"),
        ("s_0011", "weather.wind.new_york"),
        ("s_0012", "weather.humidity.new_york"),
        ("s_0013", "weather.temp.london"),
        ("s_0014", "weather.wind.london"),
        ("s_0015", "weather.humidity.london"),
        ("s_0016", "weather.temp.tokyo"),
        ("s_0017", "weather.wind.tokyo"),
        ("s_0018", "weather.humidity.tokyo"),
        ("s_0019", "weather.temp.sydney"),
        ("s_0020", "weather.wind.sydney"),
        ("s_0021", "weather.humidity.sydney"),
        ("s_0022", "weather.temp.dubai"),
        ("s_0023", "weather.wind.dubai"),
        ("s_0024", "weather.humidity.dubai"),
        ("s_0025", "weather.temp.los_angeles"),
        ("s_0026", "weather.wind.los_angeles"),
        ("s_0027", "weather.humidity.los_angeles"),
        // Crypto — original 4 (Kraken feed, z-score normalized)
        ("s_0030", "crypto.price.btc"),
        ("s_0031", "crypto.volume.btc"),
        ("s_0032", "crypto.change_24h.btc"),
        ("s_0033", "crypto.price.eth"),
        ("s_0034", "crypto.volume.eth"),
        ("s_0035", "crypto.change_24h.eth"),
        ("s_0036", "crypto.price.sol"),
        ("s_0037", "crypto.volume.sol"),
        ("s_0038", "crypto.change_24h.sol"),
        ("s_0042", "crypto.price.xrp"),
        ("s_0043", "crypto.volume.xrp"),
        ("s_0044", "crypto.change_24h.xrp"),
        // Stocks — 5 tickers × 2 properties (prev-day OHLCV, z-score normalized)
        ("s_0050", "stocks.close.spy"),
        ("s_0051", "stocks.volume.spy"),
        ("s_0052", "stocks.close.nvda"),
        ("s_0053", "stocks.volume.nvda"),
        ("s_0054", "stocks.close.aapl"),
        ("s_0055", "stocks.volume.aapl"),
        ("s_0056", "stocks.close.tsla"),
        ("s_0057", "stocks.volume.tsla"),
        ("s_0058", "stocks.close.msft"),
        ("s_0059", "stocks.volume.msft"),
        // Aviation — 3 zones × 3 aggregates
        ("s_0070", "aviation.count.europe"),
        ("s_0071", "aviation.speed_avg_ms.europe"),
        ("s_0072", "aviation.altitude_avg_m.europe"),
        ("s_0073", "aviation.count.uk"),
        ("s_0074", "aviation.speed_avg_ms.uk"),
        ("s_0075", "aviation.altitude_avg_m.uk"),
        ("s_0076", "aviation.count.north_atlantic"),
        ("s_0077", "aviation.speed_avg_ms.north_atlantic"),
        ("s_0078", "aviation.altitude_avg_m.north_atlantic"),
        // Ships — 3 zones × 2 aggregates
        ("s_0090", "ships.count.north_sea"),
        ("s_0091", "ships.speed_avg_knots.north_sea"),
        ("s_0092", "ships.count.english_channel"),
        ("s_0093", "ships.speed_avg_knots.english_channel"),
        ("s_0094", "ships.count.thames"),
        ("s_0095", "ships.speed_avg_knots.thames"),
        // Earthquakes — USGS M2.5+ global feed
        ("s_0100", "earthquakes.rate"),
        ("s_0101", "earthquakes.magnitude_avg"),
        ("s_0102", "earthquakes.depth_avg_km"),
        ("s_0103", "earthquakes.significance_avg"),
        // Commodities — 6 entities (rolling z-score)
        ("s_0130", "commodities.price.brent_crude"),
        ("s_0131", "commodities.price.wti_crude"),
        ("s_0132", "commodities.price.natural_gas"),
        ("s_0133", "commodities.price.copper"),
        ("s_0134", "commodities.price.corn"),
        ("s_0135", "commodities.price.wheat"),
        // Economic — FRED indicators (fixed-range normalized)
        ("s_0140", "economic.consumer_sentiment"),
        ("s_0141", "economic.initial_jobless_claims"),
        ("s_0142", "economic.unemployment_rate"),
        ("s_0143", "economic.inflation_cpi"),
        ("s_0144", "economic.gdp_growth"),
        // Internet — Cloudflare Radar
        ("s_0150", "internet.mobile_traffic_pct"),
        // Crypto — 10 new coins (Kraken feed)
        ("s_0160", "crypto.price.doge"),
        ("s_0161", "crypto.volume.doge"),
        ("s_0162", "crypto.change_24h.doge"),
        ("s_0163", "crypto.price.ltc"),
        ("s_0164", "crypto.volume.ltc"),
        ("s_0165", "crypto.change_24h.ltc"),
        ("s_0166", "crypto.price.ada"),
        ("s_0167", "crypto.volume.ada"),
        ("s_0168", "crypto.change_24h.ada"),
        ("s_0169", "crypto.price.uni"),
        ("s_0170", "crypto.volume.uni"),
        ("s_0171", "crypto.change_24h.uni"),
        ("s_0172", "crypto.price.link"),
        ("s_0173", "crypto.volume.link"),
        ("s_0174", "crypto.change_24h.link"),
        ("s_0175", "crypto.price.atom"),
        ("s_0176", "crypto.volume.atom"),
        ("s_0177", "crypto.change_24h.atom"),
        ("s_0178", "crypto.price.pol"),
        ("s_0179", "crypto.volume.pol"),
        ("s_0180", "crypto.change_24h.pol"),
        ("s_0181", "crypto.price.avax"),
        ("s_0182", "crypto.volume.avax"),
        ("s_0183", "crypto.change_24h.avax"),
        ("s_0184", "crypto.price.near"),
        ("s_0185", "crypto.volume.near"),
        ("s_0186", "crypto.change_24h.near"),
        ("s_0187", "crypto.price.dot"),
        ("s_0188", "crypto.volume.dot"),
        ("s_0189", "crypto.change_24h.dot"),
    ];
    let mut map = serde_json::Map::new();

    // 1. Apply hardcoded overrides first (106 hand-curated entries).
    for (k, v) in entries {
        map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }

    // 2. Auto-generate readable names for every other registered signal.
    for (key, signal_id) in registry.iter() {
        let s_key = format!("s_{:04}", signal_id.0);
        if map.contains_key(&s_key) {
            continue; // hardcoded entry wins
        }
        let human = derive_human_name(key);
        map.insert(s_key, serde_json::Value::String(human));
    }

    serde_json::Value::Object(map)
}

/// Convert a registry key into a human-readable signal name.
///
/// Examples:
///   "flux-weather/new-york#temperature_c"      → "weather.temperature_c.new_york"
///   "flux-airquality/london#pm25"               → "airquality.pm25.london"
///   "flux-aviation-europe#agg.count"            → "aviation_europe.count"
///   "flux-aviation-europe#agg.mean.speed_ms"    → "aviation_europe.mean_speed_ms"
///   "flux-volcanoes#agg.fraction.color_code=ORANGE" → "volcanoes.fraction_color_code_ORANGE"
///   "internal/continuity#value"                 → already hardcoded; this branch won't fire
fn derive_human_name(registry_key: &str) -> String {
    let (left, prop_raw) = registry_key.split_once('#').unwrap_or((registry_key, ""));

    // Strip "agg." prefix; replace dots and equals with underscores for the property name.
    let prop = prop_raw
        .trim_start_matches("agg.")
        .replace('.', "_")
        .replace('=', "_");

    // Split namespace + entity (entity is absent for aggregation keys).
    let (ns_raw, entity) = match left.split_once('/') {
        Some((ns, e)) => (ns, e),
        None          => (left, ""),
    };

    // Strip "flux-" prefix and convert hyphens to underscores in both parts.
    let ns     = ns_raw.trim_start_matches("flux-").replace('-', "_");
    let entity = entity.replace('-', "_");

    if entity.is_empty() {
        format!("{}.{}", ns, prop)
    } else {
        format!("{}.{}.{}", ns, prop, entity)
    }
}

fn action_key() -> serde_json::Value {
    let entries: &[(&str, &str)] = &[
        // AdjustDecay — tune how fast a signal tracks incoming data
        ("action_100", "adjust_decay.btc_price.faster (+0.005)"),
        ("action_101", "adjust_decay.btc_price.slower (-0.005)"),
        ("action_102", "adjust_decay.eth_price.faster (+0.005)"),
        ("action_103", "adjust_decay.eth_price.slower (-0.005)"),
        ("action_104", "adjust_decay.spy_close.faster (+0.005)"),
        ("action_105", "adjust_decay.spy_close.slower (-0.005)"),
        ("action_106", "adjust_decay.europe_air_count.faster (+0.005)"),
        ("action_107", "adjust_decay.europe_air_count.slower (-0.005)"),
        ("action_108", "adjust_decay.north_sea_ship_count.faster (+0.005)"),
        ("action_109", "adjust_decay.north_sea_ship_count.slower (-0.005)"),
        // AdjustBaseline — recalibrate what counts as 'normal' for a signal
        ("action_110", "adjust_baseline.btc_price.up (+0.02)"),
        ("action_111", "adjust_baseline.btc_price.down (-0.02)"),
        ("action_112", "adjust_baseline.eth_price.up (+0.02)"),
        ("action_113", "adjust_baseline.eth_price.down (-0.02)"),
        ("action_114", "adjust_baseline.spy_close.up (+0.02)"),
        ("action_115", "adjust_baseline.spy_close.down (-0.02)"),
        ("action_116", "adjust_baseline.europe_air_count.up (+0.02)"),
        ("action_117", "adjust_baseline.europe_air_count.down (-0.02)"),
        ("action_118", "adjust_baseline.north_sea_ship_count.up (+0.02)"),
        ("action_119", "adjust_baseline.north_sea_ship_count.down (-0.02)"),
        // CoinDerivedSignal — create a new derived signal from two existing ones
        ("action_120", "coin_derived.ratio.btc_price_div_eth_price"),
        ("action_121", "coin_derived.difference.btc_price_minus_spy_close"),
        // Self-model and housekeeping
        ("action_122", "gen_action (generate corrective action from causal history)"),
        ("action_123", "write_prompt (write self-reflection prompt to disk)"),
        ("action_124", "read_prompt (read self-reflection prompt from disk)"),
        ("action_125", "reload_actions (reload actions.json from disk)"),
        // Runtime self-generated
        ("action_200+", "runtime_derived (self-generated actions coined by gen_action at runtime)"),
    ];
    let mut map = serde_json::Map::new();
    for (k, v) in entries {
        map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    serde_json::Value::Object(map)
}
