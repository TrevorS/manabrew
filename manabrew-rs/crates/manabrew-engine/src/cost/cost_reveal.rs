//! Reveal cards as a cost. Mirrors Java's `CostReveal`.

use forge_foundation::ZoneType;

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::Reveal {
        amount,
        type_filter,
        from,
        description,
    } = part
    else {
        return String::new();
    };
    let amount_text = amount.to_string();
    let i = amount.as_literal();
    let reveal_from = match from {
        super::RevealFrom::Hand => vec![ZoneType::Hand],
        super::RevealFrom::Exile => vec![ZoneType::Exile],
        super::RevealFrom::HandOrBattlefield => vec![ZoneType::Hand, ZoneType::Battlefield],
        super::RevealFrom::All => vec![
            ZoneType::Battlefield,
            ZoneType::Hand,
            ZoneType::Graveyard,
            ZoneType::Exile,
            ZoneType::Stack,
            ZoneType::Library,
            ZoneType::Command,
        ],
    };
    let mut sb = String::from("Reveal ");
    if super::cost_part::type_is_source(type_filter) {
        sb.push_str(type_filter);
    } else if type_filter == "Hand" {
        return "Reveal your hand".to_string();
    } else if type_filter == "SameColor" {
        let count = i.map_or_else(|| "null".to_string(), |i| i.to_string());
        return format!("Reveal {count} cards from your hand that share a color");
    } else {
        let desc = if type_filter == "Card" {
            "Card".to_string()
        } else {
            format!("{} card", description.as_deref().unwrap_or(type_filter))
        };
        sb.push_str(&super::convert_amount_type_to_words(i, &amount_text, &desc));
    }
    sb.push_str(&format!(
        " from your {}",
        reveal_from[0].to_string().to_lowercase()
    ));
    if reveal_from.len() > 1 {
        let desc = description.as_deref().unwrap_or(type_filter);
        sb.push_str(" or choose ");
        sb.push_str(&super::convert_amount_type_to_words(i, &amount_text, desc));
        sb.push_str(" you control");
    }
    sb
}

// NOTE: pay_as_decided is handled by GameLoop::pay_reveal_cost() in game_action.rs
// because it requires agent interaction for card selection.
// Java's CostReveal.doPayment() calls game.getAction().reveal() which is display-only.

pub const HASH_LKI: &str = "Revealed";
pub const HASH_CARDS: &str = "RevealedCards";

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
    let super::CostPart::Reveal {
        amount,
        type_filter,
        from,
        ..
    } = part
    else {
        return false;
    };
    let resolved_amount = amount.resolve(game, source, player);
    if type_filter == "Hand" {
        return true;
    }
    if type_filter == "CARDNAME" || type_filter == "NICKNAME" {
        let src_zone = game.card(source).zone;
        return match from {
            super::RevealFrom::Hand => src_zone == forge_foundation::ZoneType::Hand,
            super::RevealFrom::Exile => src_zone == forge_foundation::ZoneType::Exile,
            super::RevealFrom::HandOrBattlefield => {
                src_zone == forge_foundation::ZoneType::Hand
                    || src_zone == forge_foundation::ZoneType::Battlefield
            }
            super::RevealFrom::All => true,
        };
    }
    let candidates = super::reveal_candidates(game, player, source, type_filter, from);
    if type_filter == "SameColor" {
        for &cid in &candidates {
            let color = game.card(cid).color;
            let count = candidates
                .iter()
                .filter(|&&other| game.card(other).color.shares_color_with(color))
                .count() as i32;
            if count >= resolved_amount {
                return true;
            }
        }
        return false;
    }
    (candidates.len() as i32) >= resolved_amount
}
