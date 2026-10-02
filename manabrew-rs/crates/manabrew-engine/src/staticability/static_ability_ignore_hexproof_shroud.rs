use forge_foundation::ZoneType;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::ids::PlayerId;
use crate::parsing::CompiledSelector;
use crate::staticability::StaticMode;

pub fn ignore_hexproof(game: &GameState, target: &Card, activator: PlayerId) -> bool {
    any_ignore(game, target, activator, StaticMode::IgnoreHexproof)
}

pub fn ignore_shroud(game: &GameState, target: &Card, activator: PlayerId) -> bool {
    any_ignore(game, target, activator, StaticMode::IgnoreShroud)
}

fn any_ignore(game: &GameState, target: &Card, activator: PlayerId, mode: StaticMode) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone == ZoneType::Battlefield || c.zone == ZoneType::Command)
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_mode(&mode))
        {
            if let Some(valid) = st_ab.ir.activator_raw.as_deref() {
                if !valid_filter::matches_valid_player_selector_in_game(
                    &crate::parsing::cached_compiled_selector(valid),
                    activator,
                    source,
                    source.controller,
                    game,
                ) {
                    continue;
                }
            }
            if !matches_valid_entity(st_ab.ir.valid_entity.as_ref(), target, source) {
                continue;
            }
            return true;
        }
    }
    false
}

fn matches_valid_entity(valid: Option<&CompiledSelector>, target: &Card, source: &Card) -> bool {
    valid_filter::matches_valid_card_selector_opt(valid, target, source)
}
