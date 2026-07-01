# Task 10 — Registry-Keyed Perception Action Targets

**Status**: implemented, awaiting deploy
**Date**: 2026-05-10
**File changed**: `observer-gene/src/main.rs` (~30 lines)

## What changed

`build_action_space()` now takes `&SignalRegistry` as a parameter and resolves the
five perception-target signal IDs by semantic key instead of hardcoded `SignalId(N)` literals.

A `lookup` closure at the top of the function calls `registry.get_by_key(key)` and panics
loudly at startup if any key is absent — making key/seed drift impossible to miss.

### Five let-bindings added (replace all inline `SignalId(N)` literals)

| Variable | Registry key | Signal ID (from seed) |
|---|---|---|
| `btc_price` | `flux-crypto/bitcoin#price` | 30 |
| `eth_price` | `flux-crypto/ethereum#price` | 33 |
| `spy_close` | `flux-stocks/SPY#close` | 50 |
| `aviation_eu_cnt` | `flux-aviation-europe#agg.count` | 70 |
| `ships_ns_cnt` | `flux-ships-north-sea#agg.count` | 90 |

### Call site update (line ~298 in main)

```rust
// before
let mut action_space = build_action_space();

// after
let mut action_space = build_action_space(&registry);
```

`build_pollers` is called before `build_action_space`, so the registry is
fully populated when the lookup runs (seeded keys are present from load regardless).

## Behavior unchanged

Action space is byte-identical to pre-task-10:
- 26 actions total, IDs 100–125, same gates, same target signal IDs
- `space.next_id = 200` preserved
- No gene-core changes

## cargo check

```
11 gene-core warnings (baseline, unchanged)
0 observer-gene warnings
Finished `dev` profile in 3.90s
```

## Deploy procedure

Standard (same as prior tasks):

```bash
# On ubuntu-dev (.13)
cd /home/etl/projects/gene-observer
~/.cargo/bin/cargo build --release
rsync -avz target/release/observer-gene etl@192.168.50.107:~/observer-gene/observer-gene.new

# On etl-flux (.107)
sudo systemctl stop observer-gene
mv ~/observer-gene/observer-gene ~/observer-gene/observer-gene.bak
mv ~/observer-gene/observer-gene.new ~/observer-gene/observer-gene
chmod +x ~/observer-gene/observer-gene
sudo systemctl start observer-gene
sudo journalctl -u observer-gene -n 100 --no-pager | grep -i panic
```

## Post-deploy verification

```bash
# Service active, no panics
sudo systemctl status observer-gene

# Action IDs 100–109 still firing
sudo journalctl -u observer-gene --since "5 minutes ago" \
  | grep -E "action=10[0-9]" | head
```

## ADR 001 status

With task-10 complete, ADR 001 is fully closed:
- task-06: persistent registry ✅
- task-07: catalog + scalar/flat_multi shapes ✅
- task-08: nested_by_key/array_record/time_series shapes ✅
- task-09: aggregation shape ✅
- task-10: registry-keyed action targets ✅
