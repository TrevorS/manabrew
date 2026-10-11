use super::{resolve_numeric_svar, EffectContext};
use crate::ability::ability_ir::EffectIr;
use crate::event::RunParams;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `LifeLoseEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(LifeLoseEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let amount = resolve_life_amount(ctx, sa);
    let mut life_lost = 0;
    let mut loss_map = Vec::new();
    for target in crate::ability::spell_ability_effect::get_target_players(ctx.game, sa) {
        if crate::player::has_lost(ctx.game, target) {
            continue;
        }
        let lost = crate::player::lose_life(
            ctx.game,
            ctx.trigger_handler,
            Some(ctx.agents),
            target,
            amount,
            false,
            Some(sa),
        );
        if lost > 0 {
            loss_map.push((target, lost));
        }
        life_lost += lost;
    }

    // Set AFLifeLost SVar on source card so chained sub-abilities (e.g. GainLife) can read it.
    // Mirrors Java's `sa.setSVar("AFLifeLost", "Number$" + lifeLost)`.
    if let Some(source_id) = sa.source {
        ctx.game
            .card_mut(source_id)
            .svars
            .insert("AFLifeLost".to_string(), format!("Number${life_lost}"));
    }

    for (target, lost) in loss_map {
        ctx.trigger_handler.run_trigger(
            TriggerType::LifeLostAll,
            RunParams {
                player: Some(target),
                life_amount: Some(lost),
                source_card: sa.source,
                source_sa: Some(Box::new(sa.clone())),
                ..Default::default()
            },
            false,
        );
    }
}

fn resolve_life_amount(ctx: &EffectContext, sa: &SpellAbility) -> i32 {
    if let Some(EffectIr::LoseLife(ir)) = &sa.ir.effect {
        if let Some(amount) = &ir.amount {
            let resolved = amount.resolve_for_spell_ability(ctx.game, sa, 1);
            #[cfg(debug_assertions)]
            debug_assert_eq!(
                resolved,
                resolve_numeric_svar(ctx.game, sa, crate::parsing::keys::LIFE_AMOUNT, 1),
                "compiled LoseLife amount diverged from string params"
            );
            return resolved;
        }
    }

    resolve_numeric_svar(ctx.game, sa, crate::parsing::keys::LIFE_AMOUNT, 1)
}
