use super::EffectContext;
use crate::ability::spell_ability_effect::get_defined_cards_or_targeted;
use crate::parsing::{raw_get, raw_has_key};

/// Mirrors Java's `CleanupEffect.java`.
///
/// `DB$ Cleanup | ClearRemembered$ True`
///
/// Clears remembered cards and CMC values from the source card.
/// Used at the end of transform trigger chains (e.g. Delver of Secrets).
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `CleanupEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(CleanupEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let source_id = if raw_has_key(&sa.ability_text, "Defined") {
        match get_defined_cards_or_targeted(ctx.game, sa).first().copied() {
            Some(card_id) => card_id,
            None => return,
        }
    } else {
        match sa.source {
            Some(card_id) => card_id,
            None => return,
        }
    };

    if sa.ir.clear_remembered {
        ctx.game.card_mut(source_id).clear_remembered();
    }
    if let Some(defined) = raw_get(&sa.ability_text, "ForgetDefined") {
        let forgotten = crate::ability::ability_utils::get_defined_cards(
            ctx.game,
            Some(source_id),
            defined,
            Some(sa.activating_player),
        );
        for card_id in forgotten {
            ctx.game.card_mut(source_id).remove_remembered(card_id);
        }
    }
    if raw_has_key(&sa.ability_text, "ClearImprinted") {
        ctx.game.card_mut(source_id).clear_imprinted_cards();
    }
    if raw_has_key(&sa.ability_text, "ClearCoinFlips") {
        ctx.game.card_mut(source_id).clear_flip_result();
    }
    if raw_has_key(&sa.ability_text, "ClearChosenCard") {
        ctx.game.card_mut(source_id).chosen_cards.clear();
    }
    if raw_has_key(&sa.ability_text, "ClearChosenPlayer") {
        ctx.game.card_mut(source_id).chosen_player = None;
    }
    if raw_has_key(&sa.ability_text, "ClearChosenType") {
        let card = ctx.game.card_mut(source_id);
        card.chosen_type = None;
        card.chosen_type2 = None;
    }
    if raw_has_key(&sa.ability_text, "ClearChosenColor") {
        ctx.game.card_mut(source_id).chosen_colors.clear();
    }
    if raw_has_key(&sa.ability_text, "ClearNamedCard") {
        ctx.game.card_mut(source_id).named_cards.clear();
    }
}
