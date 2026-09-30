use forge_foundation::ZoneType;

use super::EffectContext;

/// `DB$ Regeneration` — the regeneration shield being used: heal the creature, tap it and
/// remove it from combat.
///
/// Mirrors Java's `RegenerationEffect.java`, which is a different effect from
/// `RegenerateEffect` (that one hands out the shield).
#[manabrew_engine_macros::spell_effect(RegenerationEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let host_immutable = sa
        .source
        .is_some_and(|host| ctx.game.card(host).is_immutable());
    for card_id in crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa) {
        if ctx.game.card(card_id).zone != ZoneType::Battlefield {
            continue;
        }
        ctx.game.card_mut(card_id).clear_damage();
        ctx.game.tap(card_id);
        if let Some(combat) = ctx.combat.as_deref_mut() {
            combat.remove_from_combat(card_id, ctx.game);
        }
        if host_immutable {
            let card = ctx.game.card_mut(card_id);
            card.regeneration_shields = (card.regeneration_shields - 1).max(0);
            if let Some(host) = sa.source {
                ctx.game.card_mut(host).remove_remembered(card_id);
            }
        }
    }
}
