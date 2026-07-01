/// Multi-namespace Flux WebSocket subscriber.
///
/// Single WS connection handles all observer-gene feeds:
/// weather, crypto, stocks, aviation, ships, commodities, economic, internet.
/// Earthquakes are handled separately by gene-core's FluxPoller.
///
/// Threading pattern: identical to gene-core/src/signal/flux.rs — do not deviate.
/// - std::thread::spawn with its own tokio current_thread runtime
/// - Arc<Mutex<MultiFluxState>> shared between WS task and pollers
/// - try_lock() in pollers — skip if WS task is mid-write
/// - Reconnect loop: on error, sleep 10s, retry

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::shape::EntityFilter;

/// Shared state: entity_id → property_name → most recent JSON value.
/// Covers all observer-gene namespaces except flux-earthquakes.
pub struct MultiFluxState {
    pub entities:  HashMap<String, HashMap<String, Value>>,
    pub connected: bool,
    pub filter:    EntityFilter,
}

impl MultiFluxState {
    /// Construct with an explicit filter (preferred path).
    pub fn with_filter(filter: EntityFilter) -> Self {
        Self { entities: HashMap::new(), connected: false, filter }
    }

    /// Legacy constructor kept for Default impl. Accepts all `flux-*` entities
    /// plus `knowledge-gene/steer`, matching the old `is_handled()` behavior.
    pub fn new() -> Self {
        Self::with_filter(EntityFilter {
            include_prefixes: vec!["flux-".to_string()],
            exclude_prefixes: vec![],
            include_exact: std::iter::once("knowledge-gene/steer".to_string()).collect(),
        })
    }
}

impl Default for MultiFluxState {
    fn default() -> Self { Self::new() }
}

/// Spawn the background WebSocket task on a dedicated OS thread with its own
/// tokio runtime — same pattern as gene-core/src/signal/flux.rs.
pub fn spawn_flux_multi_ws_task(
    flux_url: String,
    state:    Arc<Mutex<MultiFluxState>>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("flux multi ws tokio runtime");
        rt.block_on(run_ws(flux_url, state));
    });
}

// ── Background WebSocket task ─────────────────────────────────────────────────

async fn run_ws(url: String, state: Arc<Mutex<MultiFluxState>>) {
    loop {
        tracing::info!("flux multi ws: connecting to {}", url);
        match connect_and_listen(url.clone(), state.clone()).await {
            Ok(()) => tracing::info!("flux multi ws: connection closed cleanly"),
            Err(e) => tracing::warn!("flux multi ws: error: {}", e),
        }
        if let Ok(mut s) = state.lock() {
            s.connected = false;
        }
        tracing::info!("flux multi ws: reconnecting in 10s");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

async fn connect_and_listen(
    url:   String,
    state: Arc<Mutex<MultiFluxState>>,
) -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await?;

    // Subscribe to all entities; route client-side by entity_id prefix
    ws.send(Message::Text(
        r#"{"type":"subscribe","entity_id":"*"}"#.to_string().into(),
    )).await?;

    if let Ok(mut s) = state.lock() {
        s.connected = true;
    }
    tracing::info!("flux multi ws: subscribed to all entities");

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(json) = serde_json::from_str::<Value>(&text) {
                    handle_message(&json, &state);
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

fn handle_message(json: &Value, state: &Arc<Mutex<MultiFluxState>>) {
    let msg_type = match json.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None    => return,
    };

    match msg_type {
        "state_update" => {
            let entity_id = match json.get("entity_id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None     => return,
            };
            let property = match json.get("property").and_then(|v| v.as_str()) {
                Some(p) => p,
                None    => return,
            };
            let value = match json.get("value") {
                Some(v) => v.clone(),
                None    => return,
            };
            if entity_id == "knowledge-gene/steer" {
                tracing::info!(
                    "steer ws recv: property={} value={}",
                    property,
                    value,
                );
            }
            // Filter check and insert under a single lock acquisition
            if let Ok(mut s) = state.lock() {
                if s.filter.accepts(entity_id) {
                    s.entities
                        .entry(entity_id.to_string())
                        .or_default()
                        .insert(property.to_string(), value);
                }
            }
        }
        "entity_deleted" => {
            let entity_id = match json.get("entity_id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None     => return,
            };
            if let Ok(mut s) = state.lock() {
                if s.filter.accepts(entity_id) {
                    s.entities.remove(entity_id);
                }
            }
        }
        _ => {} // metrics_update and unknown types silently ignored
    }
}

