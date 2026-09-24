//! Draw cards as a cost. Mirrors Java's `CostDraw`.

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;

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
/// Mirrors Java's `CostDraw.payAsDecided()`.
pub fn pay_as_decided(
    game: &mut GameState,
    payer: PlayerId,
    source: CardId,
    ability: Option<&SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::Draw { amount, .. } = part else {
        return false;
    };
    let c = amount.resolve_for_sa(game, source, payer, ability);
    for p in get_potential_players(game, payer, source, ability, part) {
        for _ in 0..c {
            game.draw_card(p);
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
    pay_as_decided(game, player, source, None, part)
}
