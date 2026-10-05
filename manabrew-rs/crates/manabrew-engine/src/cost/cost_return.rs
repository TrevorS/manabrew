//! Return permanents to hand as a cost. Mirrors Java's `CostReturn`.

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::CardId;

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::Return {
        amount,
        type_filter,
        description,
    } = part
    else {
        return String::new();
    };
    let mut sb = String::from("Return ");
    let mut pronoun = "its";
    if super::cost_part::type_is_source(type_filter) {
        sb.push_str(type_filter);
    } else {
        let desc = super::cost_part::descriptive_type(type_filter, description.as_deref());
        match amount.as_literal() {
            Some(i) => {
                sb.push_str(&super::convert_int_and_type_to_words(i, &desc));
                if i > 1 {
                    pronoun = "their";
                }
            }
            None => sb.push_str(&super::convert_amount_type_to_words(
                None,
                &amount.to_string(),
                &desc,
            )),
        }
        sb.push_str(" you control");
    }
    sb.push_str(&format!(" to {pronoun} owner's hand"));
    sb
}

pub fn pay_as_decided_self(game: &mut GameState, source: CardId) -> bool {
    let owner = game.card(source).owner;
    game.move_card(source, ZoneType::Hand, owner);
    true
}

pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId]) -> bool {
    for &cid in cards {
        let owner = game.card(cid).owner;
        game.move_card(cid, ZoneType::Hand, owner);
    }
    true
}

pub const HASH_LKI: &str = "Returned";
pub const HASH_CARDS: &str = "ReturnedCards";

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
    let super::CostPart::Return {
        amount,
        type_filter,
        ..
    } = part
    else {
        return false;
    };
    if type_filter == "CARDNAME" {
        return game.card(source).zone == ZoneType::Battlefield;
    }
    let targets = super::get_sacrifice_targets_for_cost(game, player, type_filter, source, ability);
    (targets.len() as i32) >= amount.resolve(game, source, player)
}
