//! ControlExchange effect — swap control of two permanents.
//!
//! Ported 1:1 from Java's `ControlExchangeEffect.java`.
//! Exchange control of two target/defined permanents.

use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ids::CardId;
use crate::parsing::keys;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ControlExchangeEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ControlExchangeEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let tgts: Vec<CardId> = if sa.uses_targeting() {
        sa.target_chosen.all_target_cards()
    } else {
        Vec::new()
    };
    let mut object1 = tgts.first().copied();
    let mut object2 = None;

    if let Some(defined) = sa.defined() {
        let cards: Vec<CardId> = if let Some(uid_str) = defined.strip_prefix("CardUID_") {
            uid_str
                .parse::<u32>()
                .ok()
                .map(CardId)
                .into_iter()
                .collect()
        } else {
            crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                ctx.game, sa, defined,
            )
        };
        object2 = cards.first().copied();
        if cards.len() > 1 && !sa.uses_targeting() {
            object1 = cards.get(1).copied();
        }
    } else if tgts.len() > 1 {
        object2 = tgts.get(1).copied();
    }

    let (Some(card1), Some(card2)) = (object1, object2) else {
        return;
    };

    let c1 = ctx.game.card(card1);
    let c2 = ctx.game.card(card2);
    if c1.zone != ZoneType::Battlefield || c2.zone != ZoneType::Battlefield {
        return;
    }
    if c1.phased_out || c2.phased_out {
        return;
    }
    if !c2.can_be_controlled_by(c1.controller) || !c1.can_be_controlled_by(c2.controller) {
        return;
    }

    // Optional$
    if sa.is_optional() {
        let controller = sa.activating_player;
        let name1 = c1.card_name.clone();
        let name2 = c2.card_name.clone();
        ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let confirm = ctx.agents[controller.index()].confirm_action(
            controller,
            Some("ControlExchange"),
            &format!("Exchange control of {name1} and {name2}?"),
            &[],
            None,
            None,
        );
        if !confirm {
            return;
        }
    }

    // Swap controllers
    let player1 = ctx.game.card(card1).controller;
    let player2 = ctx.game.card(card2).controller;

    for (card, new_controller, old_controller) in
        [(card2, player1, player2), (card1, player2, player1)]
    {
        ctx.game.change_controller(card, new_controller);
        if old_controller != new_controller {
            ctx.trigger_handler.run_trigger(
                crate::trigger::TriggerType::ChangesController,
                crate::event::RunParams {
                    card: Some(card),
                    player: Some(new_controller),
                    original_controller: Some(old_controller),
                    ..Default::default()
                },
                false,
            );
        }
    }

    // RememberExchanged$
    if sa.param_is_true(keys::REMEMBER_EXCHANGED) {
        if let Some(sid) = sa.source {
            ctx.game.card_mut(sid).add_remembered_card(card1);
            ctx.game.card_mut(sid).add_remembered_card(card2);
        }
    }
}
