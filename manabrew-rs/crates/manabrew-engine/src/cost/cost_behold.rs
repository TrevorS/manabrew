//! Behold (reveal from hand or battlefield) as a cost. Mirrors Java's `CostBehold`.
//!
//! Behold extends CostReveal in Java, allowing reveal from Hand or Battlefield.
//! Optionally exiles the revealed cards.

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::CardId;

pub fn to_string(part: &super::CostPart) -> String {
    match part {
        super::CostPart::Behold {
            amount,
            type_filter,
            description,
            ..
        } => format!(
            "Behold {}",
            super::convert_amount_type_to_words(
                amount.as_literal(),
                &amount.to_string(),
                description.as_deref().unwrap_or(type_filter),
            )
        ),
        _ => String::new(),
    }
}

/// Execute behold payment for selected cards.
/// Cards have already been chosen by the agent.
/// If `exile` is true, moves revealed cards to exile.
pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId], exile: bool) -> bool {
    if exile {
        for &cid in cards {
            let owner = game.card(cid).owner;
            game.move_card(cid, ZoneType::Exile, owner);
        }
    }
    // Non-exile behold just reveals — no zone change needed
    true
}

pub const HASH_LKI: &str = "Beheld";
pub const HASH_CARDS: &str = "BeheldCards";

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::Behold {
        amount,
        type_filter,
        ..
    } = part
    else {
        return false;
    };
    let resolved_amount = amount.resolve(game, source, player);
    if type_filter.ends_with("ChosenType") {
        let mut cards = game.cards_in_zone(ZoneType::Hand, player).to_vec();
        cards.extend(
            game.cards_in_zone(ZoneType::Battlefield, player)
                .iter()
                .copied(),
        );
        for &cid in &cards {
            let shared = cards
                .iter()
                .filter(|&&other| super::shares_creature_type(game, cid, other))
                .count() as i32;
            if shared >= resolved_amount {
                return true;
            }
        }
        return false;
    }
    let mut count = 0i32;
    for &cid in game.cards_in_zone(ZoneType::Hand, player) {
        if cid == source {
            continue;
        }
        if type_filter == "Card"
            || type_filter.is_empty()
            || crate::cost::is_valid_cost_card(
                game,
                game.card(cid),
                type_filter,
                game.card(source),
                player,
                ability,
            )
        {
            count += 1;
        }
    }
    for &cid in game.cards_in_zone(ZoneType::Battlefield, player) {
        if type_filter == "Card"
            || type_filter.is_empty()
            || crate::cost::is_valid_cost_card(
                game,
                game.card(cid),
                type_filter,
                game.card(source),
                player,
                ability,
            )
        {
            count += 1;
        }
    }
    count >= resolved_amount
}
