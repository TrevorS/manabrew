//! `BecomesTarget` / `BecomesTargetOnce` trigger emission.
//!
//! Mirrors the `BecomesTarget` block of Java's `MagicStack.add`.

use crate::event::RunParams;
use crate::game::GameState;
use crate::ids::CardId;
use crate::spellability::SpellAbility;
use crate::trigger::handler::TriggerHandler;
use crate::trigger::TriggerType;

use super::effect_context::EffectContext;

pub(crate) fn emit_targeting_triggers(
    ctx: &mut EffectContext,
    card_id: CardId,
    trigger_sa: &SpellAbility,
) {
    emit_targeting_triggers_for_sa(ctx.trigger_handler, ctx.game, card_id, trigger_sa);
}

/// Keep in sync with the `BecomesTarget` block of `MagicStack.add`: every distinct
/// target of the ability and its sub-abilities (`getAllTargetChoices`) fires
/// `BecomesTarget`, then `BecomesTargetOnce` fires once for the lot.
pub(crate) fn emit_targeting_triggers_for_sa(
    trigger_handler: &mut TriggerHandler,
    game: &mut GameState,
    card_id: CardId,
    trigger_sa: &SpellAbility,
) {
    let controller = trigger_sa.activating_player;
    let mut target_cards: Vec<CardId> = Vec::new();
    let mut target_players: Vec<crate::ids::PlayerId> = Vec::new();
    let mut target_spells: Vec<u32> = Vec::new();
    let mut node = Some(trigger_sa);
    while let Some(sa) = node {
        for target_id in sa.target_chosen.all_target_cards() {
            if !target_cards.contains(&target_id) {
                target_cards.push(target_id);
            }
        }
        for target_id in sa.target_chosen.all_target_players() {
            if !target_players.contains(&target_id) {
                target_players.push(target_id);
            }
        }
        if let Some(entry_id) = sa.target_chosen.target_stack_entry {
            if !target_spells.contains(&entry_id) {
                target_spells.push(entry_id);
            }
        }
        node = sa.get_sub_ability();
    }

    for &target_id in &target_cards {
        let first_time = !game.card(target_id).has_become_target_this_turn();
        let valiant = game.card(target_id).is_valiant(controller);
        game.card_mut(target_id)
            .add_target_from_this_turn(controller);
        trigger_handler.run_trigger(
            TriggerType::BecomesTarget,
            RunParams {
                card: Some(target_id),
                target_card: Some(target_id),
                cards: Some(vec![target_id]),
                cause_player: Some(controller),
                cause_card: Some(card_id),
                source_sa: Some(trigger_sa.clone()),
                first_time: Some(first_time),
                valiant: Some(valiant),
                ..Default::default()
            },
            false,
        );
    }
    for &target_id in &target_players {
        trigger_handler.run_trigger(
            TriggerType::BecomesTarget,
            RunParams {
                player: Some(target_id),
                target_player: Some(target_id),
                cause_player: Some(controller),
                cause_card: Some(card_id),
                source_sa: Some(trigger_sa.clone()),
                ..Default::default()
            },
            false,
        );
    }
    for &entry_id in &target_spells {
        let Some(entry) = game.stack.find_by_id(entry_id) else {
            continue;
        };
        trigger_handler.run_trigger(
            TriggerType::BecomesTarget,
            RunParams {
                target_sa: Some(entry.spell_ability.clone()),
                cause_player: Some(controller),
                cause_card: Some(card_id),
                source_sa: Some(trigger_sa.clone()),
                ..Default::default()
            },
            false,
        );
    }
    if !target_cards.is_empty() || !target_players.is_empty() || !target_spells.is_empty() {
        trigger_handler.run_trigger(
            TriggerType::BecomesTargetOnce,
            RunParams {
                card: target_cards.first().copied(),
                target_card: target_cards.first().copied(),
                cards: (!target_cards.is_empty()).then(|| target_cards.clone()),
                player: target_players.first().copied(),
                target_player: target_players.first().copied(),
                cause_player: Some(controller),
                cause_card: Some(card_id),
                source_sa: Some(trigger_sa.clone()),
                ..Default::default()
            },
            false,
        );
    }

    commit_crime_for_sa(trigger_handler, game, controller, trigger_sa);
}

fn commit_crime_for_sa(
    trigger_handler: &mut TriggerHandler,
    game: &mut GameState,
    activator: crate::ids::PlayerId,
    sa: &SpellAbility,
) {
    if !crate::zone::magic_stack::commit_crime_check(game, activator, sa) {
        return;
    }
    crate::player::commit_crime(game, activator);
    trigger_handler.run_trigger(
        TriggerType::CommitCrime,
        RunParams {
            player: Some(activator),
            ..Default::default()
        },
        false,
    );
}
