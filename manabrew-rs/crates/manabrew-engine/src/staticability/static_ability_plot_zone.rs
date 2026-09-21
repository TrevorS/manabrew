use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::staticability::StaticMode;

pub fn plot_zone(game: &GameState, card: &Card) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in &source.static_abilities {
            if !st_ab.check_conditions_full(&StaticMode::PlotZone, source, game) {
                continue;
            }
            if valid_filter::matches_valid_card_selector_opt_in_game(
                st_ab.ir.valid_card.as_ref(),
                card,
                source,
                game,
            ) {
                return true;
            }
        }
    }
    false
}
