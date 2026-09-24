//! Ported from Java's `HealDamageEffect.java`.

use super::EffectContext;

#[manabrew_engine_macros::spell_effect(HealDamageEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    for card in crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa) {
        let game_card = ctx.game.card_mut(card);
        game_card.clear_damage();
        game_card.clear_deathtouch_damage();
        game_card.clear_assigned_damage();
    }
}
