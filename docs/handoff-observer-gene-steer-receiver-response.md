# Handoff: observer-gene Steer Receiver — Response to KG

**Date**: 2026-05-09  
**From**: observer-gene session (task-05)  
**To**: knowledge-gene operators  
**Re**: Steer pipeline status after observability deploy

---

## What Was Deployed

task-04 observability additions are now live on observer-gene (etl-flux, 192.168.50.107),
deployed 2026-05-09 16:31:18 UTC. Three changes — no behavioral effect:

1. **`flux_multi.rs`** — `info!` log on every WS `state_update` for `knowledge-gene/steer`,
   logging each property arrival individually before the mutex lock.
2. **`main.rs` `take_steer_command()`** — `info!` log when the entity is found in
   `MultiFluxState`, listing all fields present at that tick.
3. **`main.rs` reject branches** — both steer-ignore paths bumped from `debug!` to `info!`
   so they're visible at the default log level.

---

## Diagnostic Test Outcome

A synthetic steer injection was attempted via `POST /api/events` using the observer-gene
namespace token. The Flux server rejected it:

```
{"error":"Token does not have permission to write to namespace 'knowledge-gene'"}
```

**Root cause**: Flux enforces namespace write isolation — only knowledge-gene's own token
can write to `knowledge-gene/*`. The synthetic injection path is blocked from observer-gene's
host context.

**Implication**: The end-to-end steer path has not yet been exercised. The observability
instrumentation is in place and will fire on the first natural KG publish to
`knowledge-gene/steer`.

---

## No Follow-Up Code Change Needed

The observability binary is correctly deployed. No code change is pending. The next
diagnostic data will come from natural KG steer activity.

---

## Log Signatures for KG Operators

When knowledge-gene publishes a steer, grep for these on etl-flux:

```bash
sudo journalctl -u observer-gene -f | grep -iE "steer|knowledge-gene"
```

Expected sequence (order of `steer ws recv` lines is unspecified):

```
steer ws recv: property=action_ids value=[120]
steer ws recv: property=confidence value=0.95
steer ws recv: property=tick_ref value=<TICK>
steer ws recv: property=reason value="..."
steer entity present: fields=["action_ids","confidence","reason","tick_ref"]
tick N: steer candidate action=120 confidence=0.95 reason="..."
tick N: steered action 120 accepted
```

### Failure-mode interpretation

| Observed | Diagnosis |
|----------|-----------|
| Zero `steer ws recv` lines | Flux WS does not fan `knowledge.gene` stream to observer-gene's subscriber. Investigate Flux server-side stream filter or subscription handshake. |
| `steer ws recv` present, no `steer entity present` | `is_handled()` rejected or lock-and-insert broken. Check `flux_multi.rs:127-132`. |
| `steer entity present` with incomplete `fields=[...]` across many ticks | Properties arriving but not converging — possible `entity_deleted` race or partial publishes from KG. |
| `steer entity present` with full fields, no `steer candidate` | Type mismatch in `take_steer_command()` parsing. Inspect `value=` text in recv logs. |
| `steer candidate` then `steer ignored — stale` | `tick_ref` mismatch. Re-steer with a fresh tick value. |
| `steer candidate` then `steered action N accepted` or `overridden by regulation` | **End-to-end healthy.** KG steers flow through to the selector. |

---

## Static Key Entity

observer-gene publishes a human-readable key entity to Flux at startup and every ~10s:

```
entity: observer-gene/key
```

It maps signal IDs → names, action IDs → descriptions, and explains the Φ/Φ_C symbol format.
Useful for interpreting the `steer candidate` log line's `action=N` field.
