use forge_foundation::PhaseType;

use super::{resolve_numeric_svar, EffectContext};

/// Resolve `SP$ AddPhase` — add extra combat (or main) phases to the current turn.
///
/// Mirrors Java `AddPhaseEffect.java`.
///
/// # Card script examples
/// ```text
/// A:SP$ AddPhase | ExtraPhase$ Combat
/// A:SP$ AddPhase | ExtraPhase$ Combat | NumPhases$ 2
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `AddPhaseEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(AddPhaseEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let after_phase = crate::parsing::raw_get(&sa.ability_text, "AfterPhase")
        .and_then(PhaseType::from_script_name)
        .unwrap_or(ctx.game.turn.phase);
    let next_phase = after_phase.next();
    let following_extra = crate::parsing::raw_get(&sa.ability_text, "FollowedBy")
        .and_then(PhaseType::from_script_name);
    let extra = sa.ir.extra_phase_text.as_deref().unwrap_or("Combat");

    let num = resolve_numeric_svar(ctx.game, sa, "NumPhases", 1);
    for _ in 0..num {
        let mut extra_phase_list: Vec<PhaseType> = match extra {
            "Beginning" => PhaseType::BEGINNING_PHASE.to_vec(),
            "Combat" => PhaseType::COMBAT_PHASE.to_vec(),
            _ => PhaseType::from_script_name(extra).into_iter().collect(),
        };
        extra_phase_list.extend(following_extra);
        ctx.game
            .turn
            .add_extra_phase(after_phase, &extra_phase_list, next_phase);
    }
}

#[cfg(test)]
mod tests {
    use crate::ability::spell_ability_effect::SpellAbilityEffect;
    use crate::HashMap;

    use crate::ability::effects::EffectContext;
    use crate::agent::PassAgent;
    use crate::game::GameState;
    use crate::ids::PlayerId;
    use crate::mana::ManaPool;
    use crate::spellability::SpellAbility;
    use crate::trigger::handler::TriggerHandler;
    use forge_foundation::PhaseType;

    #[test]
    fn add_extra_combat_phase() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        game.turn.phase = PhaseType::Main1;

        let sa =
            SpellAbility::new_simple(None, p0, "SP$ AddPhase | ExtraPhase$ Combat | Amount$ 1");

        let mut th = TriggerHandler::new();
        let mut agents: Vec<Box<dyn crate::agent::PlayerAgent>> =
            vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut mp = vec![ManaPool::default(), ManaPool::default()];
        let templates = HashMap::default();
        let templates_variants = HashMap::default();
        let token_fallback = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter::default();
        let mut ctx = EffectContext {
            game: &mut game,
            combat: None,
            agents: &mut agents,
            trigger_handler: &mut th,
            token_templates: &templates,
            token_art_variants: &templates_variants,
            token_fallback: &token_fallback,
            edition_dates: &edition_dates,
            mana_pools: &mut mp,
            parent_target_card: None,
            rng: &mut rng_adapter,
        };
        super::AddPhaseEffect::resolve(&mut ctx, &sa);

        assert_eq!(
            ctx.game.turn.next_phase_after(PhaseType::Main1),
            PhaseType::CombatBegin
        );
        assert_eq!(
            ctx.game.turn.next_phase_after(PhaseType::CombatEnd),
            PhaseType::CombatBegin
        );
        assert_eq!(
            ctx.game.turn.next_phase_after(PhaseType::CombatEnd),
            PhaseType::Main2
        );
    }
}
