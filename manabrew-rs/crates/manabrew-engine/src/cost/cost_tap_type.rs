//! Tap other permanents of a type as a cost. Mirrors Java's `CostTapType`.

use crate::game::GameState;
use crate::ids::CardId;
use crate::spellability::SpellAbility;
use forge_foundation::{lang, CoreType};

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::TapType {
        amount,
        type_filter,
        min_total_power,
        description,
        ..
    } = part
    else {
        return String::new();
    };
    if let Some(power) = min_total_power {
        return format!("Tap any number of untapped creatures you control other than CARDNAME with total power {power} or greater");
    }
    let desc = super::cost_part::descriptive_type(type_filter, description.as_deref());
    let amt = amount.to_string();
    let mut sb = String::from("Tap ");
    if type_filter.contains("Other") {
        let rep = if type_filter.contains(".Other") {
            ".Other"
        } else {
            "+Other"
        };
        let mut desc_trim = desc.replace(rep, "");
        if CoreType::from_name(&desc_trim).is_some() {
            desc_trim = desc_trim.to_lowercase();
        }
        if amt == "1" {
            sb.push_str(&format!("another untapped {desc_trim}"));
        } else {
            sb.push_str(&lang::noun_with_numeral(
                &amt,
                &format!("other untapped {desc_trim}"),
            ));
        }
        if !desc_trim.contains("you control") {
            sb.push_str(" you control");
        }
    } else if amt == "X" {
        sb.push_str(&format!("any number of untapped {desc}s you control"));
    } else {
        sb.push_str(&lang::noun_with_numeral_except_one(
            &amt,
            &format!("untapped {desc}"),
        ));
        sb.push_str(" you control");
    }
    if type_filter.contains("sharesCreatureTypeWith") {
        sb.push_str(" that share a creature type");
    }
    sb
}

/// Effective power contributed when this card is tapped to pay a tap-type cost.
pub fn tap_power_value(game: &GameState, card: CardId, ability: Option<&SpellAbility>) -> i32 {
    let card_ref = game.card(card);
    if crate::staticability::static_ability_tap_power_value::with_toughness(game, card_ref, ability)
    {
        card_ref.toughness().max(0)
    } else {
        (card_ref.power()
            + crate::staticability::static_ability_tap_power_value::get_mod(
                game, card_ref, ability,
            ))
        .max(0)
    }
}

/// Pay by tapping the selected cards.
/// Cards are passed in (already selected by agent).
/// Mirrors Java's `CostTapType.doListPayment()`.
pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId]) -> bool {
    for &cid in cards {
        game.tap(cid);
    }
    // TODO: Fire TapAll trigger — currently done by caller
    true
}

pub fn refund(game: &mut GameState, cards: &[CardId]) {
    for &cid in cards {
        game.card_mut(cid).set_tapped(false);
    }
}

pub const HASH_LKI: &str = "Tapped";
pub const HASH_CARDS: &str = "TappedCards";

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::TapType {
        amount,
        type_filter,
        min_total_power,
        can_tap_source,
        ..
    } = part
    else {
        return false;
    };
    let targets = super::get_tap_type_targets_for_cost(
        game,
        player,
        type_filter,
        source,
        *can_tap_source,
        ability,
    );
    if let Some(power_threshold) = min_total_power {
        let total_power: i32 = targets
            .iter()
            .map(|&cid| tap_power_value(game, cid, ability))
            .sum();
        total_power >= *power_threshold
    } else {
        (targets.len() as i32) >= amount.resolve(game, source, player)
    }
}
