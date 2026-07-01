// Phase 3: shared Flux WebSocket state + spawn function
pub mod flux_multi;

// Phase 4: per-feed pollers
// weather, economic, internet moved to shape module (task-07)
// crypto, stocks, commodities moved to shape module (task-08)
// aviation, ships moved to shape module (task-09) — aggregation shape

// Earthquakes: reuse gene_core::signal::flux::{FluxPoller, spawn_flux_ws_task} directly
