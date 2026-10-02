use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::parsing::CompiledSelector;
use crate::staticability::StaticMode;

pub fn cant_attach(game: &GameState, attachment: &Card, target: &Card, check_sba: bool) -> bool {
    game.cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield)
        .any(|source| {
            source
                .static_abilities
                .iter()
                .filter(|sa| sa.check_mode(&StaticMode::CantAttach))
                .any(|st_ab| {
                    apply_cant_attach_ability(st_ab, source, attachment, target, check_sba)
                })
        })
}

pub fn apply_cant_attach_ability(
    st_ab: &crate::staticability::StaticAbility,
    source: &Card,
    attachment: &Card,
    target: &Card,
    check_sba: bool,
) -> bool {
    if !matches_valid_card(st_ab.ir.valid_card.as_ref(), attachment, source) {
        return false;
    }
    if !matches_valid_card(st_ab.ir.target.as_ref(), target, source) {
        return false;
    }
    if let Some(valid_card_to_target) = st_ab.ir.valid_card_to_target.as_ref() {
        if !matches_valid_card_for_target(attachment, valid_card_to_target, target) {
            return false;
        }
    }
    !((check_sba || !st_ab.ir.exception_sba)
        && st_ab.ir.exceptions.is_some()
        && matches_valid_card(st_ab.ir.exceptions.as_ref(), attachment, source))
}

fn matches_valid_card(valid: Option<&CompiledSelector>, card: &Card, source: &Card) -> bool {
    valid_filter::matches_valid_card_selector_opt(valid, card, source)
}

fn matches_valid_card_for_target(card: &Card, valid: &CompiledSelector, target: &Card) -> bool {
    valid.alternatives.iter().any(|alternative| {
        alternative
            .parts
            .iter()
            .map(|part| part.value.as_str())
            .all(|tok| match tok {
                "Card" | "Permanent" => true,
                "Creature" => card.is_creature(),
                "Card.Self" => card.id == target.id,
                "Self" => card.id == target.id,
                "nonLegendary" => !target.type_line.is_legendary(),
                "Legendary" => target.type_line.is_legendary(),
                _ => true,
            })
    })
}
