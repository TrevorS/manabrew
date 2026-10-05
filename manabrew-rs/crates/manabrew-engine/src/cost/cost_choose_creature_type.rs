//! Choose a creature type as a cost. Mirrors Java's `CostChooseCreatureType`.

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};

pub fn to_string(part: &super::CostPart) -> String {
    match part {
        super::CostPart::ChooseCreatureType(amount) => format!(
            "Choose {}",
            super::convert_amount_type_to_words(
                amount.as_literal(),
                &amount.to_string(),
                "creature type",
            )
        ),
        _ => String::new(),
    }
}

/// Pay by setting chosen type on the source card.
/// Mirrors Java's `CostChooseCreatureType.payAsDecided()` →
/// `sa.getHostCard().setChosenType(pd.type)`.
pub fn pay_as_decided(
    game: &mut GameState,
    source: CardId,
    _player: PlayerId,
    chosen_type: &str,
) -> bool {
    game.card_mut(source)
        .set_chosen_type(Some(chosen_type.to_string()), None, true);
    true
}

pub fn can_pay(
    _game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    _source: crate::ids::CardId,
    _player: crate::ids::PlayerId,
    _ability: Option<&crate::spellability::SpellAbility>,
    _part: &super::CostPart,
) -> bool {
    true
}
