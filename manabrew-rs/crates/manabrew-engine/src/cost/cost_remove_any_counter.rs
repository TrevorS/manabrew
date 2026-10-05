//! Remove any counter type from permanents as a cost. Mirrors Java's `CostRemoveAnyCounter`.

use crate::card::CounterType;
use crate::game::GameState;
use crate::game::TypeRegistry;
use crate::ids::CardId;
use forge_foundation::lang;

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::RemoveAnyCounter {
        amount,
        type_filter,
        counter_type,
        description,
    } = part
    else {
        return String::new();
    };
    let amount_text = amount.to_string();
    let counters = match counter_type {
        Some(counter) => format!("{} counter", counter.get_name().to_lowercase()),
        None => "counter".to_string(),
    };
    let multiple = amount_text != "1";
    let descriptive = descriptive_type(type_filter, description.as_deref(), multiple);
    let mut sb = String::from("Remove ");
    sb.push_str(&super::convert_amount_type_to_words(
        amount.as_literal(),
        &amount_text,
        &counters,
    ));
    sb.push_str(" from ");
    if super::cost_part::type_is_source(type_filter) {
        sb.push_str(&descriptive);
    } else {
        if multiple {
            sb.push_str(" among ");
        }
        sb.push_str(&descriptive);
        sb.push_str(" you control");
    }
    sb
}

pub fn descriptive_type(type_filter: &str, description: Option<&str>, multiple: bool) -> String {
    let type_desc = match description {
        Some(desc) => desc.to_string(),
        None if super::cost_part::type_is_source(type_filter) => return type_filter.to_string(),
        None => {
            let types: Vec<String> = type_filter
                .split(';')
                .map(|ty| {
                    if multiple {
                        TypeRegistry::get_plural_type(ty)
                    } else {
                        ty.to_string()
                    }
                })
                .collect();
            let types: Vec<&str> = types.iter().map(String::as_str).collect();
            lang::build_valid_desc(&types, multiple)
        }
    };
    if !multiple && !type_desc.starts_with("an") {
        let article = if lang::starts_with_vowel(&type_desc) {
            "an"
        } else {
            "a"
        };
        return format!("{article} {type_desc}");
    }
    type_desc
}

/// Pay by removing counters from selected permanents.
/// The caller provides the (card, counter_type, amount) decisions.
/// Mirrors Java's `CostRemoveAnyCounter.payAsDecided()` which iterates
/// `decision.counterTable`.
pub fn pay_as_decided(game: &mut GameState, removals: &[(CardId, CounterType, i32)]) -> bool {
    for &(cid, ref ct, amt) in removals {
        game.card_mut(cid).remove_counter(ct, amt);
    }
    true
}

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}

/// `CostRemoveAnyCounter.getMaxAmountX`: the source alone for `CARDNAME` or `NICKNAME` (`payCostFromSource`),
/// otherwise the battlefield cards valid for the type with the source as context.
pub fn valid_cards(
    game: &crate::game::GameState,
    player: crate::ids::PlayerId,
    source: CardId,
    type_filter: &str,
) -> Vec<CardId> {
    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
        return vec![source];
    }
    game.cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
        .iter()
        .copied()
        .filter(|&cid| {
            type_filter == "Permanent"
                || type_filter.is_empty()
                || type_filter.split(';').any(|filter| {
                    crate::ability::ability_utils::matches_valid_cards_for_source(
                        game,
                        source,
                        game.card(cid),
                        None,
                        filter.trim(),
                    )
                })
        })
        .collect()
}

/// FORGE BUG, mirrored under `mirror_forge_bugs`: Forge pays from the ability's host object, so a
/// triggered ability whose host has changed zones since it triggered still removes counters from
/// the old object (CR 400.7 says the new object has none).
pub fn pays_from_lki(
    game: &GameState,
    card_id: CardId,
    source: CardId,
    ability: Option<&crate::spellability::SpellAbility>,
) -> bool {
    let card = game.card(card_id);
    game.mirror_forge_bugs
        && card_id == source
        && card.lki_counters.is_some()
        && ability
            .and_then(|sa| sa.trigger_source_zone_timestamp)
            .is_some_and(|ts| ts != card.zone_timestamp)
}

pub fn counters_of<'a>(
    game: &'a GameState,
    card_id: CardId,
    source: CardId,
    ability: Option<&crate::spellability::SpellAbility>,
) -> &'a std::collections::BTreeMap<CounterType, i32> {
    let card = game.card(card_id);
    match &card.lki_counters {
        Some(lki) if pays_from_lki(game, card_id, source, ability) => lki,
        _ => &card.counters,
    }
}

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::RemoveAnyCounter {
        amount,
        type_filter,
        counter_type,
        ..
    } = part
    else {
        return false;
    };
    let total: i32 = valid_cards(game, player, source, type_filter)
        .iter()
        .map(|&cid| {
            let counters = counters_of(game, cid, source, ability);
            match counter_type {
                Some(ct) => counters.get(ct).copied().unwrap_or(0),
                None => counters.values().sum(),
            }
        })
        .sum();
    total >= amount.resolve(game, source, player)
}
