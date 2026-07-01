# Task 18: Bundled Deploy of ADR 002 — Session Summary

**Status**: Deploy runbook prepared; awaiting user execution  
**Date**: 2026-05-13  
**Scope**: Build + deploy of tasks 14, 15, 16, 17, 17a to production (.107)  
**Rollback target**: `~/observer-gene/observer-gene.bak` (task-13 binary)

---

## What This Deploys

Five tasks bundled into one binary swap:

| Task | What it brings |
|---|---|
| **14** | Per-signal adaptive deviation thresholds (`k=1.5 × stddev`, window=500) + 4th gene-core patch |
| **15** | SIG_FAMILIARITY (ID 7, weight 2.0) — anti-stagnation meta-signal |
| **16** | SIG_LEDGER_DEGENERACY (ID 6, weight 3.0) — symbol diversification meta-signal |
| **17** | SIG_PREDICTION_ERROR (ID 5, weight 5.0) — closes perception-action learning loop |
| **17a** | `signal_key()` covers all 884 registered signals (was 106) via `derive_human_name()` |

---

## Pre-Deploy Verification (Session)

### Step 1: cargo check ✅

```
warning: `gene-core` (lib) generated 11 warnings (run `cargo fix --lib -p gene-core` to apply 10 suggestions)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.71s
```

- **gene-core warnings**: 11 (unchanged baseline)
- **observer-gene warnings**: 0
- Source compiles cleanly. Proceed to release build.

---

## Deploy Runbook (User Must Execute)

**Rule**: Only the user builds and runs gene. All commands below require user execution.

### Step 1: Release Build (on ubuntu-dev .13)

```bash
cd /home/etl/projects/gene-observer
~/.cargo/bin/cargo build --release
```

Expected: 11 gene-core warnings, 0 observer-gene warnings. Exits 0.

### Step 2: Artifact Verification

```bash
# Freshness check — must show today's timestamp after source mtimes
stat -c '%y' target/release/observer-gene

# Critical strings present — expect ≥ 3
strings target/release/observer-gene | grep -cE "prediction_error|ledger_degeneracy|familiarity"

# derive_human_name baked in — expect ≥ 1
strings target/release/observer-gene | grep -c "derive_human_name"
```

### Step 3: Smoke Test (90 seconds — mandatory)

90s minimum: SIG_PREDICTION_ERROR needs HORIZON_TICKS=5000 (~64s) to mature.

```bash
export FLUX_TOKEN=<flux-namespace-token>   # redacted — set from your local secret
mkdir -p /tmp/observer-task18
./target/release/observer-gene \
    --data-dir /tmp/observer-task18 \
    --flux-url ws://192.168.50.107:3000/api/ws \
    --flux-token "$FLUX_TOKEN" 2>&1 | tee /tmp/observer-task18.log &
PID=$!
sleep 90
kill $PID
wait 2>/dev/null

# 1. Feed audit — check ~29 feed lines; flux-weather seeded=18 fresh=318
grep -E "feed=flux-" /tmp/observer-task18.log

# 2. Meta-signal slots — IDs 5/6/7 must be present in registry
jq '.by_key | to_entries | map(select(.value <= 9))' /tmp/observer-task18/signal-registry.json

# 3. Entity key expansion
strings target/release/observer-gene | grep -cE "airquality\.|wildfires\.|grid_eu\.|volcanoes\."

rm -rf /tmp/observer-task18 /tmp/observer-task18.log
```

**Expected**:
- `flux-weather`: seeded=18 fresh=318
- All other existing feeds: seeded=N fresh=0
- Registry: 8 entries with value ≤ 9 (IDs 0–7); IDs 5/6/7 = prediction_error/ledger_degeneracy/familiarity
- Strings grep: ≥ 4 (auto-generated names baked in)

**STOP if any smoke test check fails.**

### Step 4: Backup Production Data (on .107)

```bash
ssh etl@192.168.50.107 'cd ~/observer-gene && tar czf data-pre-task18-$(date +%Y%m%d).tar.gz data/'
```

### Step 5: Deploy (from .13 then .107)

```bash
# From .13
rsync -avz target/release/observer-gene \
    etl@192.168.50.107:~/observer-gene/observer-gene.new

# On .107
ssh etl@192.168.50.107
sudo systemctl stop observer-gene
mv ~/observer-gene/observer-gene     ~/observer-gene/observer-gene.bak
mv ~/observer-gene/observer-gene.new ~/observer-gene/observer-gene
chmod +x ~/observer-gene/observer-gene
sudo systemctl start observer-gene
sudo journalctl -u observer-gene -f
```

---

## Post-Deploy Verification Checklist

### Within 30 seconds

```bash
# 1. Service active, no restart loops
sudo systemctl status observer-gene

# 2. No panics
sudo journalctl -u observer-gene -n 300 --no-pager | grep -iE "panic|thread.*panicked"

# 3. Registry has IDs 0-7 all populated
jq '.by_key | with_entries(select(.value <= 9))' ~/observer-gene/data/signal-registry.json
```

### Within 2 minutes

```bash
# 4. All feeds + meta-signals in audit log
sudo journalctl -u observer-gene --since "3 minutes ago" \
  | grep -E "feed=flux-|signal registry" | head -30

# 5. Meta-signals contributing to imbalance
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fstate \
  | jq '.properties.signal_drivers | with_entries(select(.key | startswith("s_000")))'
```

### Within 5 minutes

```bash
# 6. observer-gene/key has ~884 entries
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fkey \
  | jq '.properties.signals | length'

# 7. Spot-check generated names
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fkey \
  | jq '.properties.signals | with_entries(select(.key | IN("s_0030", "s_0500", "s_0700")))'

# 8. Pattern stream diversifying
sudo journalctl -u observer-gene --since "5 minutes ago" | grep "coined" | head -20
```

---

## Rollback Criteria

Roll back if any of:
- Service panics within first 30 minutes
- `s_0000` (continuity) drops below 0.5 within 24h
- CPU > 50% sustained post-warmup
- Memory growth > 200 MB beyond pre-deploy footprint
- Any existing feed shows `fresh > 0` in audit log

### Rollback Procedure

```bash
ssh etl@192.168.50.107
sudo systemctl stop observer-gene
mv ~/observer-gene/observer-gene     ~/observer-gene/observer-gene.failed
mv ~/observer-gene/observer-gene.bak ~/observer-gene/observer-gene
sudo systemctl start observer-gene
sudo journalctl -u observer-gene -f
```

---

## 7-Day Observation Protocol

Run once per day:

```bash
# Meta-signal values
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fstate \
  | jq '.properties.signal_drivers | with_entries(select(.key | startswith("s_000")))'

# KG steer accept/override counts (last 24h)
sudo journalctl -u observer-gene --since "24 hours ago" \
  | grep "steered action" | grep -oE "accepted|overridden" | sort | uniq -c

# Symbol ledger health
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fsymbols \
  | jq '.properties | length'

# Dominant symbol cluster
curl -s http://192.168.50.107:3000/api/state/entities/observer-gene%2Fstate \
  | jq '.properties | {dominant, cluster_n: (.cluster | length)}'
```

### Expected Trajectories

| Metric | Day 0 | Day 3 | Day 7 |
|---|---|---|---|
| `s_0005` (prediction_error) | 0.0 (warmup) | 0.2–0.4 | 0.1–0.3 |
| `s_0006` (ledger_degeneracy) | 0.8+ (Φ_0231 dominant) | 0.4–0.6 | 0.2–0.5 |
| `s_0007` (familiarity) | varies | declining | low, oscillating |
| KG override rate | ~100% | 70–90% | 40–80% |
| Dominant cluster size | 7–230 (current) | smaller | smaller, more variable |
| Symbol count | 333 (current) | growing | growing |

---

## Deploy Observations (fill in after deploy)

- **Binary built**: [timestamp]
- **Artifact checks**: strings grep prediction_error|... = [N]; derive_human_name = [N]
- **Smoke test**: weather seeded=18 fresh=[N]; meta-signals [5/6/7 present?]
- **Deployed**: [timestamp]
- **Startup**: [tick resumed from]; [symbols at restart]; [first imbalance reading]
- **Registry**: by_key=[N]; IDs 5/6/7 present=[Y/N]
- **Memory**: [RSS at 5 min]
- **CPU**: [CPU% at 5 min]
