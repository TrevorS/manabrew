//! Draw cards as a cost. Mirrors Java's `CostDraw`.

use crate::agent::PlayerAgent;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::replacement::{ReplacementEvent, ReplacementResult};
use crate::spellability::SpellAbility;
use crate::trigger::{TriggerHandler, TriggerType};

pub fn get_potential_players(
    game: &GameState,
    payer: PlayerId,
    source: CardId,
    ability: Option<&SpellAbility>,
    part: &super::CostPart,
) -> Vec<PlayerId> {
    let super::CostPart::Draw {
        amount,
        type_filter,
    } = part
    else {
        return Vec::new();
    };
    let c = amount.resolve_for_sa(game, source, payer, ability);
    let selector = crate::parsing::cached_compiled_selector(type_filter);
    let fallback;
    let sa = match ability {
        Some(sa) => sa,
        None => {
            fallback = SpellAbility::new_simple(Some(source), payer, "");
            &fallback
        }
    };
    game.alive_players()
        .into_iter()
        .filter(|&p| {
            crate::player::player_property::is_valid(p, &selector, game, source, payer, sa)
                && crate::staticability::static_ability_cant_draw::can_draw_amount(game, p, c) >= c
        })
        .collect()
}

/// Pay by drawing cards.
/// Mirrors Java's `CostDraw.payAsDecided()`, which draws through `Player.drawCards`.
pub fn pay_as_decided(
    game: &mut GameState,
    mut trigger_handler: Option<&mut TriggerHandler>,
    mut agents: Option<&mut [Box<dyn PlayerAgent>]>,
    payer: PlayerId,
    source: CardId,
    ability: Option<&SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::Draw { amount, .. } = part else {
        return false;
    };
    let c = amount.resolve_for_sa(game, source, payer, ability);
    if c <= 0 {
        return true;
    }
    for p in get_potential_players(game, payer, source, ability, part) {
        let mut event = ReplacementEvent::DrawCards {
            player: p,
            count: c,
        };
        let result = match agents.as_deref_mut() {
            Some(agents) => {
                crate::replacement::apply_replacements_with_agents(game, agents, &mut event)
            }
            None => crate::replacement::apply_replacements(game, &mut event),
        };
        if matches!(
            result,
            ReplacementResult::Skipped | ReplacementResult::Replaced
        ) {
            continue;
        }
        let ReplacementEvent::DrawCards { count, .. } = event else {
            continue;
        };
        for _ in 0..count {
            let Some(card_id) = game.player_draw_one_internal(p, false, agents.as_deref_mut())
            else {
                continue;
            };
            let Some(handler) = trigger_handler.as_deref_mut() else {
                continue;
            };
            let drawn_snapshot = game.player(p).drawn_this_turn;
            handler.run_trigger(
                TriggerType::Drawn,
                crate::event::RunParams {
                    card: Some(card_id),
                    player: Some(p),
                    drawn_this_turn_snapshot: Some(drawn_snapshot),
                    ..Default::default()
                },
                false,
            );
            if handler.has_number_drawn_triggers(game) {
                handler.flush_waiting_triggers(game);
            }
        }
    }
    true
}

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    !get_potential_players(game, player, source, ability, part).is_empty()
}

pub fn pay_with_decision(
    game: &mut GameState,
    player: PlayerId,
    source: crate::ids::CardId,
    part: &super::CostPart,
    _decision: &crate::cost::payment_decision::PaymentDecision,
) -> bool {
    pay_as_decided(game, None, None, player, source, None, part)
}
