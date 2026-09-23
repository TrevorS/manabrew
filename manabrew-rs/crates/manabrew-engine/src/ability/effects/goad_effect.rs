use forge_foundation::ZoneType;

use super::EffectContext;
use crate::spellability::AbilityDuration;

/// `SP$ Goad` — goad target creature(s). Goaded creatures must attack each
/// combat if able, and can't attack the player who goaded them.
///
/// Mirrors Java's `GoadEffect.java`.
///
/// # Card script examples
/// ```text
/// A:SP$ Goad | ValidTgts$ Creature.OppCtrl
/// A:SP$ Goad | Defined$ Valid Creature.OppCtrl
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `GoadEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(GoadEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let player = sa.activating_player;
    let remember = crate::parsing::raw_has_key(&sa.ability_text, "RememberGoaded");
    let ungoad = crate::parsing::raw_has_key(&sa.ability_text, "NoLonger");
    let duration = sa
        .ir
        .duration
        .clone()
        .unwrap_or(AbilityDuration::UntilYourNextTurn);

    for card in crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa) {
        if ctx.game.card(card).zone != ZoneType::Battlefield {
            continue;
        }
        if ungoad {
            ctx.game.card_mut(card).un_goad();
            continue;
        }
        ctx.game.card_mut(card).add_goad(player);
        if duration != AbilityDuration::Permanent {
            crate::ability::spell_ability_effect::add_until_command(
                ctx.game,
                Some(&duration),
                player,
                sa.source,
                crate::phase::PhaseCommand::RemoveGoad { card, player },
            );
        }
        if remember && ctx.game.card(card).goaded_by.is_some() {
            if let Some(source) = sa.source {
                ctx.game.card_mut(source).add_remembered_card(card);
            }
        }
    }
}
