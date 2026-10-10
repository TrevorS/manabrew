use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;

/// `SP$ RemoveFromCombat` — remove target creature from combat.
///
/// Mirrors Java's `RemoveFromCombatEffect.java`.
///
/// # Card script examples
/// ```text
/// A:SP$ RemoveFromCombat | ValidTgts$ Creature
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `RemoveFromCombatEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(RemoveFromCombatEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let remember = crate::parsing::raw_has_key(&sa.ability_text, "RememberRemovedFromCombat");
    let unblock_defined =
        crate::parsing::raw_get(&sa.ability_text, "UnblockCreaturesBlockedOnlyBy")
            .map(str::to_string);
    let mut targets = crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa);
    if targets.is_empty() && matches!(sa.defined_ref(), Some(DefinedRef::ParentTarget)) {
        targets.extend(ctx.parent_target_card);
    }
    for card_id in targets {
        if !ctx.game.turn.phase.is_combat() || ctx.game.card(card_id).zone != ZoneType::Battlefield
        {
            continue;
        }
        let Some(combat) = ctx.combat.as_deref_mut() else {
            continue;
        };
        if let Some(defined) = unblock_defined.as_deref() {
            if let Some(&blocker) =
                crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game, sa, defined,
                )
                .first()
            {
                for attacker in combat.get_attackers_for(blocker) {
                    if combat
                        .get_blockers_for(attacker)
                        .iter()
                        .all(|&b| b == blocker)
                    {
                        combat.blocked_attackers.remove(&attacker);
                    }
                }
            }
        }
        combat.save_lki(card_id);
        combat.remove_from_combat(card_id, ctx.game);
        if remember {
            if let Some(source) = sa.source {
                ctx.game
                    .host_object_mut(source, sa)
                    .add_remembered_card(card_id);
            }
        }
    }
}
