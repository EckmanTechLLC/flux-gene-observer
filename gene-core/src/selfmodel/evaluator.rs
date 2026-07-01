use crate::regulation::action::ActionSpace;
use crate::regulation::causal::CausalTracer;
use crate::regulation::scorer::ImbalanceScorer;
use crate::regulation::selector::ActionSelector;
use crate::selfmodel::meta::MetaSignal;
use crate::selfmodel::model::SelfModel;
use crate::signal::bus::SignalBus;
use crate::symbol::activation::SymbolActivationFrame;

pub struct ActionEvaluator {
    pub self_model_weight: f64,
}

impl ActionEvaluator {
    pub fn new() -> Self {
        Self { self_model_weight: 0.3 }
    }

    pub fn select(
        &self,
        bus: &SignalBus,
        space: &ActionSpace,
        causal: &CausalTracer,
        selector: &mut ActionSelector,
        self_model: &SelfModel,
        meta: &MetaSignal,
        frame: &SymbolActivationFrame,
        urgency: f64,
        tick: u64,
        steered_action: Option<u32>,
    ) -> Option<u32> {
        let current_symbols: Vec<u32> = frame.active.iter().map(|(i, _, _)| *i).collect();
        let all_action_ids: Vec<u32> = space.all().iter().map(|a| a.id).collect();

        let self_model_pref = self_model.preferred_action(&current_symbols, &all_action_ids);

        // Pass tick through to selector so cooldowns work
        let reg_choice = selector.select(bus, space, causal, urgency, tick);

        // ── Steering: validate and optionally override ────────────────────────
        // A steered action wins when confidence ≥ 0.80 (checked by caller) and
        // it passes continuity protection + cooldown checks. Continuity is
        // absolute — nothing overrides it. System actions are also excluded
        // (the regulation selector handles those).
        if let Some(steer_id) = steered_action {
            if let Some(action) = space.get(steer_id) {
                let safe = !ImbalanceScorer::harms_continuity(bus, action)
                    && !action.is_system_action();
                if safe {
                    let reg_pref = reg_choice.and_then(|id| {
                        self_model.action_preferences.get(&id).copied()
                    }).unwrap_or(0.0);
                    let steer_pref = self_model.action_preferences.get(&steer_id)
                        .copied()
                        .unwrap_or(0.5); // assume moderate preference when unknown
                    // Regulation wins only if it has a clearly higher known preference
                    let reg_clearly_better = reg_choice.is_some()
                        && reg_pref > steer_pref + 0.2
                        && self_model.action_preferences.contains_key(
                            &reg_choice.unwrap()
                        );
                    if !reg_clearly_better {
                        return Some(steer_id);
                    }
                }
            }
        }

        match (self_model_pref, reg_choice) {
            (Some(sm_id), Some(reg_id)) => {
                if let Some(action) = space.get(sm_id) {
                    if ImbalanceScorer::harms_continuity(bus, action) {
                        return Some(reg_id);
                    }
                    // Never let self-model force a system action — selector handles those
                    if action.is_system_action() {
                        return Some(reg_id);
                    }
                }

                let sm_pref  = self_model.action_preferences.get(&sm_id).copied().unwrap_or(0.0);
                let reg_pref = self_model.action_preferences.get(&reg_id).copied().unwrap_or(0.0);

                let use_self_model = sm_pref > reg_pref
                    && meta.confidence > 0.6
                    && self_model.action_preferences.get(&sm_id).is_some();

                if use_self_model { Some(sm_id) } else { Some(reg_id) }
            }
            (Some(sm_id), None) => {
                // Only use self-model preference if it's not a system action
                if space.get(sm_id).map(|a| a.is_system_action()).unwrap_or(false) {
                    None
                } else {
                    Some(sm_id)
                }
            }
            (None, reg) => reg,
        }
    }
}

impl Default for ActionEvaluator {
    fn default() -> Self { Self::new() }
}
