//! Discard cards as a cost. Mirrors Java's `CostDiscard`.
//!
//! Java's `CostDiscard` extends `CostPartWithList` and uses `doPayment()`
//! to call `payer.discard(targetCard, ...)`. It also fires `DiscardedAll`
//! trigger after all discards.

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::Discard {
        amount,
        type_filter,
        description,
    } = part
    else {
        return String::new();
    };
    let amount_text = amount.to_string();
    let i = amount.as_literal();
    let mut sb = String::from("Discard ");
    if super::cost_part::type_is_source(type_filter) {
        sb.push_str(type_filter);
    } else if type_filter == "Hand" {
        sb.push_str("your hand");
    } else if type_filter == "LastDrawn" {
        sb.push_str("the last card you drew this turn");
    } else if type_filter.contains("+WithDifferentNames") {
        sb.push_str(&super::convert_amount_type_to_words(
            i,
            &amount_text,
            "Card",
        ));
        sb.push_str(" with different names");
    } else {
        let desc = if type_filter == "Card" || type_filter == "Random" {
            "card".to_string()
        } else {
            format!(
                "{} card",
                super::cost_part::descriptive_type(type_filter, description.as_deref())
            )
        };
        sb.push_str(&super::convert_amount_type_to_words(i, &amount_text, &desc));
        if type_filter == "Random" {
            sb.push_str(" at random");
        }
    }
    sb
}

/// Execute discard of self (CARDNAME).
/// Mirrors Java's `CostDiscard.doPayment()` for self-discard.
pub fn pay_as_decided_self(game: &mut GameState, source: CardId, player: PlayerId) -> bool {
    let owner = game.card(source).owner;
    game.move_card(source, ZoneType::Graveyard, owner);
    let _ = player;
    // TODO: Fire Discarded trigger — currently done by caller
    true
}

/// Execute typed discard (non-self).
/// Cards to discard are passed in (already selected by agent).
/// Mirrors Java's `CostDiscard.doPayment()`.
pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId], _player: PlayerId) -> bool {
    for &cid in cards {
        let owner = game.card(cid).owner;
        game.move_card(cid, ZoneType::Graveyard, owner);
        // TODO: Fire Discarded trigger per card
    }
    // TODO: Fire DiscardedAll trigger after all discards
    true
}

/// Hash keys for LKI/card tracking lists.
pub const HASH_LKI: &str = "Discarded";
pub const HASH_CARDS: &str = "DiscardedCards";

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
    let super::CostPart::Discard {
        type_filter,
        amount,
        ..
    } = part
    else {
        return false;
    };
    let card = game.card(source);
    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
        return card.zone == ZoneType::Hand;
    }
    if type_filter == "Hand" {
        return true;
    }
    if type_filter == "Card" || type_filter.is_empty() {
        let mut hand_size = game.cards_in_zone(ZoneType::Hand, player).len() as i32;
        if card.zone == ZoneType::Hand && card.owner == player {
            hand_size -= 1;
        }
        return hand_size >= amount.resolve(game, source, player);
    }
    let mut matching = game
        .cards_in_zone(ZoneType::Hand, player)
        .iter()
        .filter(|&&cid| is_valid_discard(game, cid, type_filter, source))
        .count() as i32;
    if card.zone == ZoneType::Hand
        && card.owner == player
        && is_valid_discard(game, source, type_filter, source)
    {
        matching -= 1;
    }
    matching >= amount.resolve(game, source, player)
}

pub fn is_valid_discard(
    game: &GameState,
    card_id: CardId,
    type_filter: &str,
    source: CardId,
) -> bool {
    if type_filter == "Random" || type_filter.contains('X') {
        return true;
    }
    type_filter.split(';').any(|valid| {
        crate::card::valid_filter::matches_valid_card_selector_in_game(
            &crate::parsing::cached_compiled_selector(valid),
            game.card(card_id),
            game.card(source),
            game,
        )
    })
}
