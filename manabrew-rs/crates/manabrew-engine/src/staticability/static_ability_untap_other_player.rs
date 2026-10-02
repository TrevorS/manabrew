use std::sync::Arc;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::ids::PlayerId;
use crate::staticability::StaticMode;

pub fn untap(cards: &[Arc<Card>], card: &Card, player: PlayerId, game: &GameState) -> bool {
    for source in cards.iter().filter(|c| c.zone.is_static_ability_source()) {
        for st_ab in source.static_abilities.iter().filter(|sa| {
            sa.check_mode(&StaticMode::UntapOtherPlayer) && sa.zones_check(source.zone)
        }) {
            if apply_untap_ability(st_ab, card, source, player, game) {
                return true;
            }
        }
    }
    false
}

pub fn apply_untap_ability(
    st_ab: &crate::staticability::StaticAbility,
    card: &Card,
    source: &Card,
    player: PlayerId,
    game: &GameState,
) -> bool {
    if !valid_filter::matches_valid_card_selector_opt_in_game(
        st_ab.ir.valid_card.as_ref(),
        card,
        source,
        game,
    ) {
        return false;
    }
    if !valid_filter::matches_valid_player_selector_opt_in_game(
        st_ab.ir.valid_player.as_ref(),
        player,
        source,
        source.controller,
        game,
    ) {
        return false;
    }
    true
}
