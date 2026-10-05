//! Move exiled cards to graveyard as a cost. Mirrors Java's `CostExiledMoveToGrave`.

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::CardId;

pub fn to_string(part: &super::CostPart) -> String {
    match part {
        super::CostPart::ExiledMoveToGrave {
            amount,
            type_filter,
            description,
        } => format!(
            "Put {} from exile into that player's graveyard",
            super::convert_amount_type_to_words(
                amount.as_literal(),
                &amount.to_string(),
                description.as_deref().unwrap_or(type_filter),
            )
        ),
        _ => String::new(),
    }
}

/// Execute the exiled-move-to-grave payment for selected cards.
/// Mirrors Java's `CostExiledMoveToGrave.doPayment()`.
pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId]) -> bool {
    for &cid in cards {
        let owner = game.card(cid).owner;
        game.move_card(cid, ZoneType::Graveyard, owner);
    }
    true
}

pub const HASH_LKI: &str = "MovedToGrave";
pub const HASH_CARDS: &str = "MovedToGraveCards";

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
    let super::CostPart::ExiledMoveToGrave {
        amount,
        type_filter,
        ..
    } = part
    else {
        return false;
    };
    let exiled = super::get_exiled_targets(game, type_filter, source, player).len() as i32;
    exiled >= amount.resolve(game, source, player)
}
