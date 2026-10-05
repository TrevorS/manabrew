//! Sacrifice permanents as a cost. Mirrors Java's `CostSacrifice`.
//!
//! Java's `CostSacrifice` extends `CostPartWithList` and uses `doListPayment()`
//! to call `game.getAction().sacrifice()`. The LKI/card tracking lists are
//! managed by the `CostPartWithList` base class.

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use forge_foundation::{lang, CoreType};

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::Sacrifice {
        amount,
        type_filter,
        description,
    } = part
    else {
        return String::new();
    };
    let amount_text = amount.to_string();
    let mut sb = String::from(if amount_text == "X" {
        "You may sacrifice "
    } else {
        "Sacrifice "
    });
    if super::cost_part::type_is_source(type_filter) {
        match description {
            Some(desc) if desc.starts_with("this") => sb.push_str(desc),
            _ => sb.push_str(type_filter),
        }
    } else if amount_text == "X" {
        let type_desc = type_filter.to_lowercase().replace(';', "s and/or ");
        sb.push_str(&format!("any number of {type_desc}s"));
    } else {
        let desc = match description {
            Some(desc) => desc.clone(),
            None if type_filter == "Permanent" || CoreType::from_name(type_filter).is_some() => {
                type_filter.to_lowercase()
            }
            None => type_filter.clone(),
        };
        if desc.starts_with("another") {
            sb.push_str(&desc);
        } else {
            sb.push_str(&lang::noun_with_numeral_except_one(&amount_text, &desc));
        }
    }
    sb
}

/// Execute the sacrifice cost payment.
/// Mirrors Java's `CostSacrifice.doListPayment()` → `game.getAction().sacrifice()`.
///
/// For "CARDNAME" sacrifice, moves the source to graveyard.
/// For typed sacrifice, the caller must have already selected cards via the agent.
///
/// NOTE: Trigger firing (Sacrificed) must be handled by the caller since it
/// requires access to the trigger handler.
pub fn pay_as_decided_self(game: &mut GameState, source: CardId, player: PlayerId) -> bool {
    let owner = game.card(source).owner;
    game.move_card(source, ZoneType::Graveyard, owner);
    let _ = player;
    // TODO: Fire Sacrificed trigger — currently done by caller in game_action.rs
    true
}

/// Execute typed sacrifice (non-self).
/// Cards to sacrifice are passed in as `cards` (already selected by agent).
/// Mirrors Java's `CostSacrifice.doListPayment()`.
pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId], _player: PlayerId) -> bool {
    for &cid in cards {
        let owner = game.card(cid).owner;
        game.move_card(cid, ZoneType::Graveyard, owner);
        // TODO: Fire Sacrificed trigger per card
    }
    true
}

/// Hash keys for LKI/card tracking lists.
/// Mirrors Java's `CostSacrifice.getHashForLKIList()` / `getHashForCardList()`.
pub const HASH_LKI: &str = "Sacrificed";
pub const HASH_CARDS: &str = "SacrificedCards";

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
    let super::CostPart::Sacrifice {
        type_filter,
        amount,
        ..
    } = part
    else {
        return false;
    };
    let card = game.card(source);
    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
        if card.zone != ZoneType::Battlefield {
            return false;
        }
        return !crate::staticability::static_ability_cant_sacrifice::cant_sacrifice(
            game, card, ability, true,
        );
    }
    if type_filter.eq_ignore_ascii_case("All") {
        let targets =
            super::get_sacrifice_targets_for_cost(game, player, type_filter, source, ability);
        return !targets.is_empty();
    }
    let valid = ability
        .and_then(|sa| super::cost_part::get_max_amount_x(game, sa, player, part, true))
        .unwrap_or_else(|| {
            super::get_sacrifice_targets_for_cost(game, player, type_filter, source, ability).len()
                as i32
        });
    valid >= amount.resolve(game, source, player)
}
