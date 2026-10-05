//! Mill cards as a cost. Mirrors Java's `CostMill`.

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::PlayerId;

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::Mill(amount) = part else {
        return String::new();
    };
    let plural = if amount.as_literal().is_none_or(|i| i > 1) {
        "s"
    } else {
        ""
    };
    format!("Mill {amount} card{plural}")
}

/// Pay by milling cards (library -> graveyard).
/// Mirrors Java's `CostMill.payAsDecided()`.
/// NOTE: Trigger firing (Milled, zone change) must be handled by the caller.
pub fn pay_as_decided(
    game: &mut GameState,
    player: PlayerId,
    amount: i32,
) -> Vec<crate::ids::CardId> {
    let mut milled = Vec::new();
    for _ in 0..amount {
        if let Some(top) = game.take_top_card_from_zone(ZoneType::Library, player) {
            game.move_card(top, ZoneType::Graveyard, player);
            milled.push(top);
        }
    }
    milled
}

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    _ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::Mill(amount) = part else {
        return false;
    };
    let resolved = amount.resolve(game, source, player);
    let lib_size = game.zone(ZoneType::Library, player).len() as i32;
    lib_size > resolved
}
