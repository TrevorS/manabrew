use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::staticability::StaticMode;

pub fn can_activate(game: &GameState, card: &Card) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in source.static_abilities.iter().filter(|sa| {
            sa.check_mode(&StaticMode::ActivateAbilityAsIfHaste) && sa.zones_check(source.zone)
        }) {
            if apply_can_activate_ability(st_ab, card, source) {
                return true;
            }
        }
    }
    false
}

fn apply_can_activate_ability(
    st_ab: &crate::staticability::StaticAbility,
    card: &Card,
    source: &Card,
) -> bool {
    valid_filter::matches_valid_card_selector_opt(st_ab.ir.valid_card.as_ref(), card, source)
}
