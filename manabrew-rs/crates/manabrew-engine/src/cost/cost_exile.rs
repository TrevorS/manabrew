//! Exile cards as a cost. Mirrors Java's `CostExile`.
//!
//! Covers ExileFromHand, ExileFromGrave, ExileFromTop, ExileSameGrave,
//! and ExileFromBattlefield variants. Java uses `zoneMode` to distinguish;
//! Rust uses separate CostPart variants for some (ExileFromAnyGrave, ExileFromSameGrave).

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::CardId;
use forge_foundation::{lang, CoreType};

pub fn to_string(part: &super::CostPart) -> String {
    let (amount, type_filter, description, from, zone_restriction) = match part {
        super::CostPart::Exile {
            amount,
            type_filter,
            from,
            description,
        } => (amount, type_filter, description, vec![*from], 1),
        super::CostPart::ExileFromAnyGrave {
            amount,
            type_filter,
            description,
        } => (
            amount,
            type_filter,
            description,
            vec![ZoneType::Graveyard],
            -1,
        ),
        super::CostPart::ExileFromSameGrave {
            amount,
            type_filter,
            description,
        } => (
            amount,
            type_filter,
            description,
            vec![ZoneType::Graveyard],
            0,
        ),
        super::CostPart::ExileCtrlOrGrave {
            amount,
            type_filter,
            description,
        } => (
            amount,
            type_filter,
            description,
            vec![ZoneType::Battlefield, ZoneType::Graveyard],
            1,
        ),
        _ => return String::new(),
    };
    let description = description.as_deref();
    let amount_text = amount.to_string();
    let i = amount.as_literal();
    let desc = super::cost_part::descriptive_type(type_filter, description);
    let [zone] = from.as_slice() else {
        return exile_multi_zone_cost_string(
            &amount_text,
            type_filter,
            description,
            &from,
            false,
            0,
        );
    };
    let origin = zone.to_string().to_lowercase();
    if super::cost_part::type_is_source(type_filter) {
        if origin != "battlefield" {
            return format!("Exile {type_filter} from your {origin}");
        }
        return format!("Exile {type_filter}");
    }
    if type_filter == "All" {
        return format!("Exile all cards from your {origin}");
    }
    if origin == "battlefield" {
        let amt = match amount_text.split_once('+') {
            Some((needed, _)) if i.is_none() => format!(
                "{} or more {desc}",
                lang::get_numeral(needed.parse().unwrap_or(0))
            ),
            _ => super::convert_amount_type_to_words(i, &amount_text, &desc),
        };
        let control = if amt.contains("you control") {
            ""
        } else {
            " you control"
        };
        return format!("Exile {amt}{control}");
    }
    if desc != "Card" && !desc.contains("card") {
        let whose = match zone_restriction {
            0 => "the same",
            -1 => "a",
            _ => "your",
        };
        return format!(
            "Exile {} from {whose} {origin}",
            lang::noun_with_numeral_except_one(&amount_text, &format!("{desc} card"))
        );
    }
    if zone_restriction == 0 {
        return format!(
            "Exile {} from the same {origin}",
            super::convert_amount_type_to_words(i, &amount_text, &desc)
        );
    }
    if amount_text == "X" {
        return format!("Exile any number of {desc} from your {origin}");
    }
    format!(
        "Exile {} from your {origin}",
        super::convert_amount_type_to_words(i, &amount_text, &desc)
    )
}

pub fn exile_multi_zone_cost_string(
    amount: &str,
    type_filter: &str,
    description: Option<&str>,
    from: &[ZoneType],
    for_kw: bool,
    x_min: i32,
) -> String {
    let mut sb = String::from("Exile ");
    let mut amt = if !amount.is_empty() && amount.chars().all(|c| c.is_ascii_digit()) {
        amount.parse().unwrap_or(0)
    } else {
        0
    };
    let part_type = type_filter.replace(".Other", "");
    let sing_noun = match description {
        Some(desc) => desc.to_string(),
        None if CoreType::from_name(&part_type).is_some() || part_type == "Permanent" => {
            part_type.to_lowercase()
        }
        None => part_type.clone(),
    };
    let plur_noun = if sing_noun.contains(' ') {
        sing_noun.clone()
    } else {
        lang::get_plural(&sing_noun)
    };
    if !for_kw && amt == 0 && x_min > 0 {
        amt = x_min;
    }
    let perm = sing_noun == "permanent";
    let other = if perm { "other " } else { "" };
    if amt == 1 {
        let a_noun = lang::noun_with_numeral_except_one("1", &sing_noun);
        if part_type == "Artifact" || perm {
            sb.push_str(&format!("another {sing_noun}"));
        } else {
            sb.push_str(&a_noun);
        }
        sb.push_str(&format!(" you control or {a_noun} card from "));
    } else if amt > 1 {
        sb.push_str(&format!(
            "the {} from among {other}{plur_noun} you control and/or {sing_noun} cards in ",
            lang::get_numeral(amt)
        ));
    } else {
        if x_min > 1 {
            sb.push_str("the ");
        }
        sb.push_str(&lang::get_numeral(x_min));
        sb.push_str(if for_kw { " or more " } else { " " });
        if x_min == 1 {
            sb.push_str(&format!("{other}{plur_noun} you control and/or "));
            if !perm {
                sb.push_str(&sing_noun);
            }
            sb.push_str(" cards from ");
        } else if from.len() > 1 {
            sb.push_str(&format!(
                "from among {other}{plur_noun} you control and/or cards from "
            ));
        } else {
            sb.push_str("from ");
        }
    }
    sb.push_str("your graveyard");
    sb
}

/// Execute exile of self (CARDNAME/OriginalHost).
/// Mirrors Java's `CostExile` doPayment for self-exile.
pub fn pay_as_decided_self(game: &mut GameState, source: CardId) -> bool {
    let owner = game.card(source).owner;
    game.move_card(source, ZoneType::Exile, owner);
    true
}

/// Execute typed exile (non-self).
/// Cards to exile are passed in (already selected by agent).
/// Mirrors Java's `CostExile.doListPayment()`.
pub fn pay_as_decided_cards(game: &mut GameState, cards: &[CardId]) -> bool {
    for &cid in cards {
        let owner = game.card(cid).owner;
        game.move_card(cid, ZoneType::Exile, owner);
    }
    true
}

/// Hash keys for LKI/card tracking lists.
pub const HASH_LKI: &str = "Exiled";
pub const HASH_CARDS: &str = "ExiledCards";

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
    let card = game.card(source);
    match part {
        super::CostPart::Exile {
            amount,
            type_filter,
            from,
            ..
        } => {
            if type_filter == "All" {
                return true;
            }
            if type_filter == "CARDNAME"
                || type_filter == "NICKNAME"
                || type_filter == "OriginalHost"
            {
                if card.zone != *from {
                    return false;
                }
                return !crate::staticability::static_ability_cant_exile::cant_exile(
                    game, card, ability, true,
                );
            }

            let base_filter = super::normalize_exile_base_filter(type_filter);
            let candidates: Vec<crate::ids::CardId> =
                super::get_zone_targets(game, player, *from, &base_filter, source)
                    .into_iter()
                    .filter(|&cid| {
                        !crate::staticability::static_ability_cant_exile::cant_exile(
                            game,
                            game.card(cid),
                            ability,
                            true,
                        )
                    })
                    .collect();

            let mut available = candidates.len() as i32;
            if *from == forge_foundation::ZoneType::Hand
                && card.zone == forge_foundation::ZoneType::Hand
                && card.owner == player
                && crate::cost::is_valid_cost_card(
                    game,
                    card,
                    &base_filter,
                    game.card(source),
                    player,
                    ability,
                )
            {
                available -= 1;
            }
            if let Some(n) = super::parse_exile_types_ge(type_filter) {
                let mut unique_types = std::collections::BTreeSet::new();
                for cid in &candidates {
                    for t in &game.card(*cid).type_line.core_types {
                        unique_types.insert(format!("{t:?}"));
                    }
                }
                if (unique_types.len() as i32) < n {
                    return false;
                }
            }
            if let Some(expr) = super::parse_exile_total_cmc_eq(type_filter) {
                let target = if expr.eq_ignore_ascii_case("X") {
                    None
                } else {
                    expr.parse::<i32>().ok()
                };
                if let Some(target) = target {
                    let values: Vec<i32> = candidates
                        .iter()
                        .map(|&cid| game.card(cid).mana_value())
                        .collect();
                    if !super::cmc_can_sum_to(target, &values) {
                        return false;
                    }
                }
            }
            if let Some(expr) = super::parse_exile_total_cmc_ge(type_filter) {
                let target = if expr.eq_ignore_ascii_case("X") {
                    None
                } else {
                    expr.parse::<i32>().ok()
                };
                if let Some(target) = target {
                    let total: i32 = candidates
                        .iter()
                        .map(|&cid| game.card(cid).mana_value())
                        .sum();
                    if total < target {
                        return false;
                    }
                }
            }
            if super::exile_requires_shared_card_type(type_filter) {
                if available < amount.resolve(game, source, player) {
                    return false;
                }
                let mut has_pair = false;
                for &a in &candidates {
                    for &b in &candidates {
                        if a != b && super::shares_card_type(game, a, b) {
                            has_pair = true;
                            break;
                        }
                    }
                    if has_pair {
                        break;
                    }
                }
                if !has_pair {
                    return false;
                }
            }
            available >= amount.resolve(game, source, player)
        }
        super::CostPart::ExileFromAnyGrave {
            amount,
            type_filter,
            ..
        } => {
            let base_filter = super::normalize_exile_base_filter(type_filter);
            // TriggeredNewCard refers to the card that just moved to the new
            // zone (e.g. Greenwarden's death trigger exiles Greenwarden from
            // the graveyard). Resolve to the ability's source card directly.
            if base_filter.contains("TriggeredNewCard") {
                let src = game.card(source);
                let is_eligible = src.zone == forge_foundation::ZoneType::Graveyard
                    && !crate::staticability::static_ability_cant_exile::cant_exile(
                        game, src, ability, true,
                    );
                return if is_eligible {
                    amount.resolve(game, source, player) <= 1
                } else {
                    false
                };
            }
            let count = game
                .players
                .iter()
                .flat_map(|p| game.cards_in_zone(forge_foundation::ZoneType::Graveyard, p.id))
                .filter(|&&cid| {
                    (base_filter == "Card"
                        || base_filter.is_empty()
                        || crate::cost::is_valid_cost_card(
                            game,
                            game.card(cid),
                            &base_filter,
                            game.card(source),
                            player,
                            ability,
                        ))
                        && !crate::staticability::static_ability_cant_exile::cant_exile(
                            game,
                            game.card(cid),
                            ability,
                            true,
                        )
                })
                .count() as i32;
            count >= amount.resolve(game, source, player)
        }
        super::CostPart::ExileFromSameGrave {
            amount,
            type_filter,
            ..
        } => {
            let resolved_amount = amount.resolve(game, source, player);
            let base_filter = super::normalize_exile_base_filter(type_filter);
            let mut by_owner: crate::HashMap<crate::ids::PlayerId, i32> = crate::HashMap::default();
            for p in &game.players {
                for &cid in game.cards_in_zone(forge_foundation::ZoneType::Graveyard, p.id) {
                    if base_filter == "Card"
                        || base_filter.is_empty()
                        || crate::cost::is_valid_cost_card(
                            game,
                            game.card(cid),
                            &base_filter,
                            game.card(source),
                            player,
                            ability,
                        )
                    {
                        if crate::staticability::static_ability_cant_exile::cant_exile(
                            game,
                            game.card(cid),
                            ability,
                            true,
                        ) {
                            continue;
                        }
                        let owner = game.card(cid).owner;
                        *by_owner.entry(owner).or_insert(0) += 1;
                    }
                }
            }
            !by_owner.values().all(|&v| v < resolved_amount)
        }
        _ => false,
    }
}
