use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::staticability::StaticMode;

pub fn cant_regenerate(game: &GameState, target: &Card) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield || c.zone == ZoneType::Command)
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_mode(&StaticMode::CantRegenerate))
        {
            let valid = st_ab.ir.valid_card.as_ref();
            if matches_valid_card(valid, target, source, game) {
                return true;
            }
        }
    }
    false
}

pub fn apply_cant_regenerate_ability(
    st_ab: &crate::staticability::StaticAbility,
    target: &Card,
    source: &Card,
    game: &GameState,
) -> bool {
    matches_valid_card(st_ab.ir.valid_card.as_ref(), target, source, game)
}

fn matches_valid_card(
    valid: Option<&crate::parsing::CompiledSelector>,
    card: &Card,
    source: &Card,
    game: &GameState,
) -> bool {
    valid_filter::matches_valid_card_selector_opt_in_game(valid, card, source, game)
}
