use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::staticability::StaticMode;

pub fn cant_crew(game: &GameState, card: &Card) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source() || c.id == card.id)
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|sa| sa.check_mode(&StaticMode::CantCrew) && sa.zones_check(source.zone))
        {
            if apply_cant_crew(st_ab, card, source, game) {
                return true;
            }
        }
    }
    false
}

pub fn apply_cant_crew(
    st_ab: &crate::staticability::StaticAbility,
    card: &Card,
    source: &Card,
    game: &GameState,
) -> bool {
    valid_filter::matches_valid_card_selector_opt_in_game(
        st_ab.ir.valid_card.as_ref(),
        card,
        source,
        game,
    )
}
