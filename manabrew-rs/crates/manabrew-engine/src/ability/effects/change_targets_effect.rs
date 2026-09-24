//! ChangeTargets effect — redirect a spell or ability's targets.
//!
//! Ported from Java's `ChangeTargetsEffect.java`.
//! Change the target(s) of target spell or ability.
//! In Java this introspects the MagicStack to find the targeted spell's
//! StackInstance and updates its TargetChoices. In our engine, the stack
//! stores SpellAbility instances with target_chosen fields.

use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::spellability::SpellAbility;

/// Configure the spell ability during construction.
/// Mirrors Java `ChangeTargetsEffect.buildSpellAbility` — sets the target zone
/// to Stack so that the ability targets spells on the stack.
pub fn build_spell_ability(sa: &mut SpellAbility) {
    if sa.uses_targeting() {
        if let Some(ref mut tr) = sa.target_restrictions {
            tr.tgt_zone = vec![ZoneType::Stack];
        }
    }
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ChangeTargetsEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ChangeTargetsEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let controller = sa.activating_player;

    let Some(target_spell_card) = get_target_spell_card(ctx, sa) else {
        return;
    };

    if sa.ir.optional {
        let card_name = ctx.game.card(target_spell_card).card_name.clone();
        ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[controller.index()].confirm_action(
            controller,
            None,
            &format!("Do you want to change targets for {card_name}?"),
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }

    // Handle RandomTarget mode: pick a random new legal target
    if sa.param_is_true(keys::RANDOM_TARGET) {
        // Find all creatures/permanents on battlefield as candidates
        let candidates: Vec<CardId> = ctx
            .game
            .cards
            .iter()
            .filter(|c| c.zone == ZoneType::Battlefield)
            .map(|c| c.id)
            .collect();

        if candidates.is_empty() {
            return;
        }

        // Pick random new target
        let idx = ctx.rng.next_int(candidates.len() as i32) as usize % candidates.len();
        let new_target = candidates[idx];

        // Update the spell's target on the stack
        // In our simplified stack model, we update the card's svar to track new target
        ctx.game
            .card_mut(target_spell_card)
            .set_s_var("RedirectedTarget", format!("{}", new_target.0));
        return;
    }

    // Handle DefinedMagnet mode: redirect to a specific permanent
    if let Some(magnet_def) = sa.ir.defined_magnet_text.as_deref() {
        let magnet_ref = DefinedRef::parse(magnet_def);
        let new_target = if matches!(magnet_ref, DefinedRef::SelfCard) {
            sa.source
        } else if matches!(magnet_ref, DefinedRef::ParentTarget) {
            sa.target_chosen.target_card
        } else {
            sa.source
                .and_then(|sid| ctx.game.card(sid).remembered_cards.first().copied())
        };

        if let Some(new_tgt) = new_target {
            ctx.game
                .card_mut(target_spell_card)
                .set_s_var("RedirectedTarget", format!("{}", new_tgt.0));
        }
    }
}

fn get_target_spell_card(ctx: &EffectContext, sa: &SpellAbility) -> Option<CardId> {
    let candidates: Vec<CardId> = if sa.uses_targeting() {
        sa.target_chosen.target_card.into_iter().collect()
    } else {
        sa.defined()
            .map(|defined| {
                crate::ability::ability_utils::get_defined_spell_abilities(defined, sa, ctx.game)
            })
            .unwrap_or_default()
            .into_iter()
            .filter_map(|spell| spell.source)
            .collect()
    };
    candidates
        .into_iter()
        .find(|&card| ctx.game.card(card).zone == ZoneType::Stack)
}
