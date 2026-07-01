# Task 02 Summary: Steer Command Race Hardening

**Date**: 2026-05-09
**Status**: Complete
**Risk**: Low — additive validation only

## Problem

Flux delivers each property of a published entity as a separate `state_update`
WebSocket message. If a tick fires between the arrival of `action_ids` and the
arrival of `tick_ref`/`confidence`, the old `take_steer_command()` would:

1. Find `action_ids` present → proceed
2. Default missing `tick_ref` to `0` and `confidence` to `0.0`
3. Remove the entity (consuming it)
4. Caller flags command stale/low-confidence and discards it
5. Late-arriving fields re-create a partial entity with no `action_ids`
6. **Steer is permanently lost** until knowledge-gene publishes again

## Fix Applied

**File**: `observer-gene/src/main.rs` — `take_steer_command()` (lines 1008–1029)

Changed `confidence` and `tick_ref` from `unwrap_or(default)` to `?` (returning
`None` without touching the entity if either field is absent or wrong type).
`action_ids` similarly uses `?` on both `get()` and `as_array()` before
extracting. `reason` remains optional (`unwrap_or("")`).

The entity is only removed after **all three required fields** (`action_ids`,
`confidence`, `tick_ref`) are confirmed present and correctly typed. Partial
entities are left untouched so the next tick can re-attempt once the remaining
fields arrive.

## Behavior Matrix

| Entity state | Old behavior | New behavior |
|---|---|---|
| All 4 fields present | consume + use | consume + use *(unchanged)* |
| `action_ids` missing/empty | return None, leave entity | return None, leave entity *(unchanged)* |
| `action_ids` present, `confidence` missing | **consume + discard (lost)** | **return None, leave entity (recovers)** |
| `action_ids` present, `tick_ref` missing | **consume + discard (lost)** | **return None, leave entity (recovers)** |
| `reason` missing | consume with empty string | consume with empty string *(unchanged)* |
| Field present, wrong type | **defaults, may misjudge** | **return None, leave entity** |
| Entity absent entirely | return None | return None *(unchanged)* |

## Verification

```
cargo check
```

Result: clean build — 11 pre-existing gene-core warnings, 0 observer-gene warnings.

## Next Step

Proceed to task-03 (deploy steering to .107). The deploy task should also
update `.odin/memory.md` to document the third local gene-core patch
(`selfmodel/evaluator.rs` — `steered_action` parameter), which is currently
undocumented.
