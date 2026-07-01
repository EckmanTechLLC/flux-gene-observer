# ADR 002 — Prediction-Oriented Learning Loop

**Status**: accepted (2026-05-13)
**Author**: foundation session, in collaboration with @matt
**Related**: ADR 001 (Dynamic Signal Registry), CLAUDE.md "Continuity (IDs 0–2)" section, OBSERVER_GENE.md

---

## Context

After ADR 001's deploy, observer-gene runs against ~884 signals across 25
Flux namespaces. Two observations from production:

1. **Symbol formation is collapsing into single clusters.** One dominant
   symbol (Φ_0231 for hours, then Φ_0330) absorbs almost all activation.
   The pattern extractor's global deviation threshold of 0.02 fires
   primarily on high-volatility signals (crypto, fire counts), while slow
   signals (economic indicators, ISS altitude) rarely produce patterns
   despite carrying real information.

2. **Perception meta-actions never execute.** 24-hour sample: 26 KG steer
   candidates, **26 overrides** (100%). Investigation (see
   `.odin/memory.md`) shows this is structurally correct: KG suggests
   `AdjustDecay`/`AdjustBaseline` on world signals (weight=0), which by
   ADR 001's design can never reduce imbalance. Gene's regulation engine
   correctly learns these actions have no causal value and prefers
   runtime-allocated CoinDerivedSignal actions instead.

These two findings have a shared root cause: **observer's optimization
objective (imbalance reduction via continuity signals) doesn't reward the
actions that would improve its perception or symbol vocabulary.** The
action loop is decorative; gene picks actions because the architecture
requires it, but action selection doesn't influence the stated output
(the symbol ledger).

The user has clarified the actual long-term goal: **the system should
eventually predict events.** Observer is meant to learn and build a
discriminating symbol vocabulary, not just passively observe. The current
design supports observation; it doesn't close a learning loop on
perception quality.

---

## Decision

Add three new weighted meta-signals that measure aspects of "learning
progress," plus one improvement to the pattern extractor. Together they
close the learning loop without changing the regulation algorithm.

### The unifying insight

Gene's existing regulation engine optimizes imbalance — a weighted sum of
signal deviations from baseline. If we add new signals that *measure*
learning progress and weight them positively, the existing causal
expected-improvement machinery automatically pursues their minimization.

No new selector. No new optimizer. Just three new signals on the bus.

### Three new meta-signals (IDs 5, 6, 7)

IDs 5–9 are free in the seed registry (the gap between continuity/internal
0–4 and world signals 10+). Use 5, 6, 7 for the new meta-signals.

#### SIG_PREDICTION_ERROR (ID 5, weight 5.0)

For every bus signal, maintain an EMA-extrapolation predictor:
- One float of EMA-smoothed value
- One float of EMA-smoothed trend (rate of change)
- Prediction at horizon h ticks: `value + h × trend`

Each tick, for each signal:
1. Use the current predictor state to make a prediction for `t + 5000` ticks
2. When tick `t + 5000` arrives, compare the prediction against actual bus value
3. Squared error contributes to a rolling-window error track (window: last 100 errors)
4. Each signal's normalized error = `tanh(error / 0.1)` mapped to [0, 1]

The meta-signal value = **mean of all signals' normalized errors**, mapped to [0, 1].

Falls when gene's predictions are accurate; rises when they're not.

**Per-signal cost**: ~50 ns/tick for EMA update + prediction. Aggregate cost at 884
signals × 78 Hz: ~1% of one CPU core. Negligible.

**The closed loop on AdjustDecay**: bus signal decay rate affects how the bus value
tracks underlying world data. The predictor reads bus value, so its accuracy depends
on decay being appropriately tuned. AdjustDecay actions that better match decay to
a signal's true rate-of-change reduce prediction error → reduce imbalance → become
learned-preferred. KG's steering on `AdjustDecay` finally has meaningful causal
consequence.

#### SIG_LEDGER_DEGENERACY (ID 6, weight 3.0)

Compute symbol activation entropy over a recent window (last 1000 ticks):
- For each symbol Φ_i, count its activations in the window: `n_i`
- Compute probability: `p_i = n_i / sum(n_i)`
- Entropy: `H = -Σ p_i × log(p_i)`
- Normalize against `log(N_active)` where `N_active` is the count of symbols with `n_i > 0`
- Degeneracy = `1 - (H / log(N_active))`, clamped to [0, 1]

High when one symbol dominates (current production state: Φ_0231 / Φ_0330 absorbs
most activation). Low when activation spreads across many symbols.

**The closed loop on symbol diversification**: gene's regulation, optimizing to
reduce this signal, prefers actions that activate quiet symbols or coin new
derived signals that participate in non-dominant clusters. AdjustBaseline on a
quiet signal so it starts crossing the deviation threshold becomes a useful move.
CoinDerivedSignal that creates a signal feeding underused patterns becomes
preferred over coining signals that reinforce the dominant cluster.

#### SIG_FAMILIARITY (ID 7, weight 2.0)

Track the last K=500 observed pattern_ids (or signal-state hashes). Each tick:
- Compute current pattern_id from the active signal set
- Familiarity = `count(current_id in last K) / K`, clamped to [0, 1]

High when the current state has been visited many times recently (well-explored
region). Low when the state is novel.

**The closed loop on stagnation**: gene minimizing this signal pursues novelty.
When the world's empirical state matches a state seen 200 times in the last 500
ticks, familiarity is 0.4 → gene's regulation prefers actions that move it away.
When the state is fresh, familiarity is near 0 → gene is content to dwell and
learn. This is intrinsic motivation expressed in the existing imbalance
framework.

### Per-signal adaptive deviation thresholds

The pattern extractor currently uses a global `deviation_threshold = 0.02`. With
heterogeneous signal volatility (crypto change vs. ISS altitude), this misses
patterns in slow signals and over-fires on volatile ones. This is a major
contributor to single-cluster collapse.

Replace with per-signal adaptive thresholds: `threshold_i = k × rolling_stddev(signal_i, window=N)`
with `k = 1.5` and `N = 500` ticks.

Per-signal threshold means each signal fires patterns when *it* deviates
significantly relative to *its own* normal range. Slow signals catch their small
absolute deviations; volatile signals stop dominating the pattern stream.

### Gene-core touches (two patches added — total now 5)

| Patch | What |
|---|---|
| `pattern/extractor.rs::current_active()` | Signature change: take `&HashMap<SignalId, f64>` for per-signal thresholds instead of a single `f64`. Backward-incompatible internally but contained. |
| `symbol/ledger.rs` | Add `pub fn entropy(&self, window: u64, current_tick: u64) -> f64` — small addition. Optional; can compute observer-side instead. |

Predictors and familiarity tracking are observer-side (new module).

Gene-core patch list grows from 3 to 5. All five tracked in `.odin/memory.md`,
none propagated upstream.

---

## Consequences

### Wins

- **Stagnation directly penalized** via SIG_FAMILIARITY. Gene actively seeks
  novelty when in well-explored territory.
- **Symbol ledger health becomes a tracked goal** via SIG_LEDGER_DEGENERACY.
  Gene learns to diversify symbol activation.
- **Perception meta-actions have closed causal feedback** via
  SIG_PREDICTION_ERROR. AdjustDecay finally means something.
- **KG steering becomes meaningfully advisory.** When KG's suggested action
  reduces prediction error or improves ledger health, causal tracer learns
  the action has value. The 100% override rate naturally relaxes when KG's
  suggestions are well-aligned.
- **Pattern detection becomes signal-aware** via adaptive thresholds. Slow
  signals stop being invisible; volatile signals stop dominating.
- **The substrate for event prediction emerges**. With prediction error
  tracked, individual-signal forecasts exist. Composite predictions (e.g.,
  "Φ_NNNN will activate in next K ticks") become a natural follow-up
  ADR — built on the per-signal predictors this ADR establishes.

### Costs

- **Two new gene-core patches** (5 total now, was 3). All small, additive,
  tracked, do-not-propagate.
- **Memory**: ~360 KB for predictors + error windows + familiarity history.
  Trivial.
- **CPU**: ~1% additional on the regulation engine's host. Negligible.
- **Behavior shift**: gene's action selection patterns will change after
  deploy. Existing causal traces remain valid (action IDs unchanged) but
  the learned preferences will re-converge to incorporate the new objectives.
  Expect 1–7 days of adjustment before behavior stabilizes.
- **Risk of objective conflict**: the four weighted contributors (continuity
  100 vs. prediction/degeneracy/familiarity 10 combined) keep
  self-preservation dominant by 10×. If we observed continuity signals
  declining post-deploy, we'd know the weights need rebalancing. Monitor
  via journalctl.

### Non-goals

- **Heavyweight predictors.** EMA extrapolation is enough to close the
  loop. Adding ML predictors (RNNs, attention, anything trained) is out of
  scope for this ADR. Future work if simple predictors prove insufficient.
- **Direct event prediction.** This ADR establishes the per-signal
  prediction substrate. Predicting symbol activations, regime transitions,
  or specific events is a separate later ADR that consumes this
  infrastructure.
- **Replacing the override threshold tuning.** Don't lower the `+0.2`
  margin in `evaluator.rs`. Let the new meta-signals do the work; if KG's
  suggestions become genuinely useful, the override rate falls naturally
  through causal learning.

---

## Implementation sequence

Five tasks, each shippable independently. Ordering chosen so each
deployment produces visible behavior change and the simpler pieces land
first.

| Task | Scope | Why this order |
|---|---|---|
| **task-14**: per-signal adaptive deviation thresholds | gene-core patch + observer-side rolling-stddev tracker | Cheapest. Immediate visible effect on symbol diversity. Lands first because subsequent tasks benefit from richer pattern detection. |
| **task-15**: SIG_FAMILIARITY | observer-side pattern-history tracker + new bus signal | Simplest meta-signal. Tests the "register weighted meta-signal" pattern before the heavier two. Direct anti-stagnation effect visible quickly. |
| **task-16**: SIG_LEDGER_DEGENERACY | optional gene-core `entropy()` + observer-side computation + new bus signal | Builds on task-15's pattern. Engages the symbol layer. Behavioral effect: gene starts diversifying symbol activations. |
| **task-17**: SIG_PREDICTION_ERROR | observer-side `predict` module + per-signal EMA predictors + error tracking + new bus signal | Heaviest piece. Lands last so it can build on the other learning signals being live. After this, perception meta-actions have full closed-loop causality. |
| **task-18**: bundled deploy + observation period | build + rsync + swap + 7-day behavioral observation | Single deploy of the four prior tasks. Watch symbol ledger entropy, KG override rate, prediction error trajectory, continuity stability. |

Each of tasks 14–17 is independently testable via `cargo check` and the
audit-log pattern from ADR 001. None requires re-running the others to
verify.

---

## Design invariants preserved

| Invariant | Status |
|---|---|
| World signals weight=0 | ✅ Unchanged. New meta-signals are observer-internal, not world signals. |
| Continuity dominant (sum weight 100) | ✅ New signals total weight 10. Continuity preserves 10× margin. |
| Symbol IDs 0–189 frozen | ✅ Meta-signals use IDs 5–7 which are seed-reserved gaps. No shift. |
| gene-core changes minimal | ✅ Two new patches (5 total), both additive, both tracked. |
| Catalog-driven feed model | ✅ Unchanged. Meta-signals are registered alongside continuity, not via catalog. |
| Observer's autonomy on continuity | ✅ Unchanged. New signals add learning objectives alongside self-preservation; they don't override it. |
| Symbol ledger persists across deploys | ✅ Existing signal IDs unchanged; ledger pattern_ids continue to resolve correctly. |

---

## Alternatives considered and rejected

- **Lowering the steer override threshold** (`+0.2` margin in evaluator.rs).
  Would let KG influence observer more directly but breaches the autonomy
  invariant. Rejected.
- **Making some world signals have non-zero weight** so AdjustDecay on them
  affects imbalance. Breaches the world-observer identity. Rejected.
- **Replacing the regulation engine** with a different algorithm (deep RL,
  evolutionary strategies, etc.). Massive scope, breaks gene-core's design
  philosophy, throws away the working continuity machinery. Rejected.
- **Heavier predictors** (LSTM, transformer-based). 1000× the cost for
  unknown benefit at this stage. Start with EMA; revisit if it proves
  insufficient. Deferred.
- **Per-signal individualized learning objectives** (each signal has its own
  prediction-error contribution to imbalance). Vastly more complex; same
  effect achievable with aggregate meta-signal at much lower complexity.
  Rejected.
- **Online curiosity-driven exploration via meta-RL**. Same intent as
  SIG_FAMILIARITY but much heavier. SIG_FAMILIARITY captures the essential
  behavior in a single weighted signal. Preferred.

---

## Open issues for follow-up

- **Predictor warmup on restart**: EMA state is lost on restart and warms
  up over ~100 ticks. Acceptable. If proves disruptive, add to checkpoint
  persistence (small follow-up).
- **Window size tuning**: 5000-tick prediction horizon, 100-error rolling
  window, 1000-tick degeneracy window, 500-tick familiarity history.
  Reasonable starting points; revisit after a week of observation if any
  behave pathologically.
- **Adaptive threshold k factor**: starting at `k = 1.5`. May need adjustment
  upward (less sensitive) if pattern stream becomes overwhelming, or
  downward (more sensitive) if certain slow signals still don't fire.
- **Interaction effects**: three new meta-signals interacting with continuity
  and meta/drive — gene's regulation may find local optima we haven't
  predicted. Monitor post-deploy. Adjust weights if any objective
  dominates.
- **Event prediction layer** (the long-term goal): with per-signal
  predictors live, composite predictions on symbol activations become
  possible. Future ADR 003 territory: predict which symbols will activate
  in the next N ticks, score those predictions, and use composite
  prediction accuracy as another meta-signal. Foundation laid here.

---

## Closing rationale

ADR 001 made observer dynamic. ADR 002 makes observer *learn*.

The architectural elegance is that the existing regulation engine doesn't
need to change. Gene already optimizes weighted signal sums; we're just
adding signals that measure aspects of learning we care about. The result
is that gene's action loop, currently decorative, becomes meaningful:
perception adjustments shape prediction quality, symbol coining shapes
ledger diversity, and stagnation is intrinsically penalized.

After deploy, expect a 1–7 day behavioral shift as gene's action
preferences re-converge to incorporate the new objectives. The symbol
ledger should diversify (degeneracy falls), action selection should
exercise more of the perception meta-action space (rather than gravitating
to CoinDerivedSignal), and KG's steering override rate should drop as
genuinely-good suggestions accumulate causal preference.

If those don't happen post-deploy, the weights or window sizes need
adjustment — not the architecture.

This ADR sets the substrate for the eventual goal of event prediction.
That goal is a future ADR built on this one's predictors.
