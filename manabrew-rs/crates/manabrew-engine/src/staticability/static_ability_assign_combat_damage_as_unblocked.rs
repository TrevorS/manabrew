use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::staticability::StaticMode;

pub fn assign_as_unblocked(game: &GameState, card: &Card, optional: bool) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield)
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_mode(&StaticMode::AssignCombatDamageAsUnblocked))
        {
            let has_optional = st_ab.ir.optional;
            if has_optional && !optional {
                continue;
            } else if !has_optional && optional {
                continue;
            }
            if matches_valid_card(st_ab.ir.valid_card.as_ref(), card, source, game) {
                return true;
            }
        }
    }
    false
}

pub fn has_optional_assign_as_unblocked(game: &GameState, card: &Card) -> bool {
    game.cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield)
        .flat_map(|source| {
            source
                .static_abilities
                .iter()
                .filter(move |sa| sa.check_mode(&StaticMode::AssignCombatDamageAsUnblocked))
                .map(move |sa| (source, sa))
        })
        .any(|(source, sa)| {
            sa.ir.optional && matches_valid_card(sa.ir.valid_card.as_ref(), card, source, game)
        })
}

pub fn has_mandatory_assign_as_unblocked(game: &GameState, card: &Card) -> bool {
    game.cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield)
        .flat_map(|source| {
            source
                .static_abilities
                .iter()
                .filter(move |sa| sa.check_mode(&StaticMode::AssignCombatDamageAsUnblocked))
                .map(move |sa| (source, sa))
        })
        .any(|(source, sa)| {
            !sa.ir.optional && matches_valid_card(sa.ir.valid_card.as_ref(), card, source, game)
        })
}

/// Java parity alias for `assign_as_unblocked`.
pub fn assign_combat_damage_as_unblocked(game: &GameState, card: &Card) -> bool {
    has_mandatory_assign_as_unblocked(game, card)
}

fn matches_valid_card(
    valid: Option<&crate::parsing::CompiledSelector>,
    card: &Card,
    source: &Card,
    game: &GameState,
) -> bool {
    valid_filter::matches_valid_card_selector_opt_in_game(valid, card, source, game)
}
