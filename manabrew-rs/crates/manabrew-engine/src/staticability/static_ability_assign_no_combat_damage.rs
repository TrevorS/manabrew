use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::staticability::StaticMode;

pub fn assign_no_combat_damage(game: &GameState, card: &Card) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield || c.zone == ZoneType::Command)
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_mode(&StaticMode::AssignNoCombatDamage))
        {
            if matches_valid_card(st_ab.ir.valid_card.as_ref(), card, source) {
                return true;
            }
        }
    }
    false
}

/// Java parity alias.
pub fn apply_assign_no_combat_damage(game: &GameState, card: &Card) -> bool {
    assign_no_combat_damage(game, card)
}

fn matches_valid_card(
    valid: Option<&crate::parsing::CompiledSelector>,
    card: &Card,
    source: &Card,
) -> bool {
    valid_filter::matches_valid_card_selector_opt(valid, card, source)
}
