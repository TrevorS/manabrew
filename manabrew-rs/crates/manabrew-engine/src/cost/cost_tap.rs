//! Tap the source permanent as a cost. Mirrors Java's `CostTap`.

use crate::game::GameState;
use crate::ids::CardId;

/// Pay the tap cost by tapping the source.
/// Mirrors Java's `CostTap.payAsDecided()`.
/// NOTE: Trigger firing (TapAll) is handled by the caller (GameLoop) since
/// it requires access to the trigger handler which is not available here.
pub fn pay_as_decided(game: &mut GameState, source: CardId) -> bool {
    game.tap(source);
    true
}

/// Refund the tap cost by untapping the source.
/// Mirrors Java's `CostTap.refund()`.
pub fn refund(game: &mut GameState, source: CardId) {
    game.card_mut(source).set_tapped(false);
}

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    _player: crate::ids::PlayerId,
    _ability: Option<&crate::spellability::SpellAbility>,
    _part: &super::CostPart,
) -> bool {
    let card = game.card(source);
    if card.tapped || card.phased_out {
        return false;
    }
    !card.is_ability_sick(&game.cards)
}
