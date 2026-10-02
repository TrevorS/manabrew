//! Give control of permanents as a cost. Mirrors Java's `CostGainControl`.

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};

/// Pay by transferring control of selected permanents.
/// Mirrors Java's `CostGainControl.doPayment()` → `card.addTempController(payer)`.
pub fn pay_as_decided_cards(
    game: &mut GameState,
    cards: &[CardId],
    new_controller: PlayerId,
) -> bool {
    for &cid in cards {
        game.card_mut(cid).controller = new_controller;
    }
    true
}

pub const HASH_LKI: &str = "ControlGained";
pub const HASH_CARDS: &str = "ControlGainedCards";

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
    let super::CostPart::GainControl {
        amount,
        type_filter,
    } = part
    else {
        return false;
    };
    let count = game
        .players
        .iter()
        .flat_map(|p| game.cards_in_zone(forge_foundation::ZoneType::Battlefield, p.id))
        .filter(|&&cid| {
            crate::cost::is_valid_cost_card(
                game,
                game.card(cid),
                type_filter,
                game.card(source),
                player,
                ability,
            )
        })
        .count() as i32;
    count >= amount.resolve(game, source, player)
}
