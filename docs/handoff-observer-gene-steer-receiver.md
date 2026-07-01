# Handoff — observer-gene steer receiver investigation

**Date:** 2026-05-09
**For:** the observer-gene repo's session
**Author:** gene-knowledge foundation session

---

## TL;DR

knowledge-gene is publishing `knowledge-gene/steer` correctly. The
entity exists in Flux. observer-gene's journal shows zero references
to knowledge-gene or steer. The receiver is either silent (no
logging) or not actually firing. Only the observer-gene session can
diagnose.

---

## What's confirmed working on the KG side

After Stage 2 deploy on .107 (2026-05-09 14:24 UTC):

- KG's systemd unit has `--steer` flag. Startup banner logs
  `mode: steering enabled`.
- ADR 001's confidence gate (≥0.80) is enforced. First post-deploy
  call at 0.78 produced no steer entity (correct behaviour).
- Second post-deploy call landed at 0.80 exactly, fired the publish.
- Entity `knowledge-gene/steer` is now in Flux. Its current state
  (verified via REST):

```
id:                knowledge-gene/steer
properties:
  action_ids:      [106, 108]
  confidence:      0.80
  tick_ref:        173030000
  reason:          "The active signal cluster reinforces a persistent
                    'Atmospheric Stagnation' regime, now explicitly
                    coupled with broad-based logistics and market
                    deceleration. Global weather stations report a
                    synchroni..."
lastUpdated:       2026-05-09T15:22:36 UTC
```

- The accompanying `knowledge-gene/state` entity at the same tick_ref
  has `suggested_actions: [106, 108]` —
  `adjust_decay.europe_air_count.faster (+0.005)` and
  `adjust_decay.north_sea_ship_count.faster (+0.005)`.

- Stream/source on the publish (per `src/steer.rs` /
  `src/publisher.rs::post_raw`): the body uses
  `stream: "knowledge.gene"` (note the dot, not slash) and
  `source: "knowledge-gene"`. The entity_id field is
  `"knowledge-gene/steer"`.

So on our side: the entity was published, received by Flux, and is
currently the latest state for that entity.

---

## What's missing on the observer-gene side

`journalctl -u observer-gene.service --since "10 minutes ago" | grep
-iE "knowledge-gene|steer"` returns **nothing** — empty result. Not a
single log line at any level mentioning either string.

That means one of:

1. **Receiver is wired but logs nothing when it processes a steer.**
   Most likely. task-04's implementation may have skipped the log
   line at the moment of receipt. Add at minimum an `info!` when the
   receiver matches `knowledge-gene/steer` and parses it.

2. **Receiver is wired but the WS message never reaches the dispatch
   path.** Flux's WS push behaviour may filter by stream namespace.
   observer-gene subscribes to all entities (its journal at startup
   says `flux ws: connected, subscribed to all entities`), but
   "all entities" might mean all of observer-gene's own namespace, not
   genuinely cross-namespace. Worth confirming by looking at the
   subscription registration code and what stream filter it uses.

3. **Receiver code isn't in the deployed binary.** The user redeployed
   observer-gene today specifically to include task-04 — so this
   is unlikely, but worth a sanity check (e.g. `strings
   /home/etl/observer-gene/observer-gene | grep -i steer`).

---

## Investigation checklist for the observer-gene session

Numbered for ease of reporting back:

1. **Confirm task-04 is in the deployed binary.**
   `strings /home/etl/observer-gene/observer-gene | grep -i
   "steer\|knowledge-gene/steer"` — should produce non-empty output
   if the receiver code is compiled in.

2. **Find the receiver code path.** Locate where `knowledge-gene/steer`
   entities are dispatched. Add unconditional info-level logging at
   the top of that handler:
   `tracing::info!("steer received: tick_ref={} action_ids={:?}
   confidence={:.2}", ...)`. This is the missing observability
   regardless of any other bug.

3. **Confirm the WS subscription is genuinely cross-namespace.**
   observer-gene's startup log says "subscribed to all entities" but
   the implementation may have a stream filter. Check the subscriber
   code for any `stream` or `namespace` filtering. If filtered,
   ensure it includes `knowledge.gene`.

4. **Re-fetch the steer entity from observer-gene's perspective.**
   Optional REST sanity check from observer-gene's process:
   `GET /api/state/entities/knowledge-gene%2Fsteer` should return the
   same JSON we see externally. Confirms Flux availability is not
   namespace-restricted at the API layer.

5. **Restart observer-gene with debug-level logging enabled** for the
   subscriber and entity-dispatch modules. The next time KG fires a
   steer (which only happens on confidence ≥ 0.80 calls), the path
   will be visible.

6. **Check ActionEvaluator integration.** ADR 001 says steered
   actions enter as a third input to `ActionEvaluator.select()`
   alongside regulation and self-model. After receipt, the steered
   action should compete in the next selection. Confirm that
   integration exists and is invoked.

7. **Stale-tick guard.** ADR 001 also says observer-gene should
   ignore steer entities where `tick_ref` is more than 100,000 ticks
   behind current. With current observer-gene ticks around the
   173,000,000 mark and KG's tick_ref matching, this shouldn't
   filter — but worth verifying the threshold logic isn't too
   aggressive.

---

## How to test end-to-end after a fix

KG publishes steer entities only when `confidence >= 0.80`. Recent
empirical confidence values: 0.65 (yesterday cycle 2), 0.95 (cycle
3), 0.72 (Stage 0 first call), 0.82 (Stage 0 cycle 2), 0.78 (Stage 1
first call), 0.80 (Stage 1 cycle 2 — fired). So roughly 1 in 2 to 1
in 3 calls fire under current regime.

To force a steer publication for testing without waiting:

- Option A: lower the threshold in KG (`src/steer.rs` —
  `apply` function gates on `interp.confidence >= 0.8`). Temporary
  for testing.
- Option B: manually publish a synthetic steer entity to Flux:

```
curl -X POST http://localhost:3000/api/events \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "stream":    "knowledge.gene",
    "source":    "knowledge-gene",
    "timestamp": NOW_MS,
    "payload": {
      "entity_id":  "knowledge-gene/steer",
      "properties": {
        "action_ids": [123],
        "confidence": 0.95,
        "tick_ref":   PASTE_CURRENT_TICK,
        "reason":     "test injection"
      }
    }
  }'
```

If observer-gene logs nothing on this synthetic injection, the
problem is definitely in observer-gene's subscription / dispatch.
If it logs something, the problem may have been just the missing
observability, and KG's actual publishes are being processed
silently.

---

## Useful references

- ADR 001 (steering protocol):
  `/home/etl/projects/gene-knowledge/docs/decisions/001-steering-protocol.md`
- KG's task-04 task prompt (steering receiver in observer-gene):
  `/home/etl/projects/gene-knowledge/.odin/tasks/task-04-steering-receiver.md`
- KG's task-04 summary:
  `/home/etl/projects/gene-knowledge/.odin/tasks/task-04-steering-receiver-summary.md`
- KG's `src/steer.rs` — sender side; for confidence-gate logic and
  exact body shape published.
- KG memory:
  `/home/etl/projects/gene-knowledge/.odin/memory.md` (current Stage
  status, Flux tokens, etc.)

---

## What KG would like back from this investigation

1. Confirmation of which root cause from the checklist (silent
   receiver, missing dispatch, missing code, stream filter, etc.).
2. Whatever observability change goes in (so future steer arrivals
   are visible without further debugging).
3. A pointer to the eventual log signature observer-gene emits when
   a steer lands, so KG operators know what to grep for.

When the receiver is logging arrivals, KG side requires no further
work — Stage 2 is then closed end-to-end.
