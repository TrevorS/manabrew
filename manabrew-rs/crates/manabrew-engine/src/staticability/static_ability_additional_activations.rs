use crate::card::valid_filter;
use crate::card::Card;
use crate::game::GameState;
use crate::ids::PlayerId;
use crate::spellability::SpellAbility;
use crate::staticability::{StaticAbility, StaticMode};

pub fn get_limit(game: &GameState, card: &Card, sa: &SpellAbility, activator: PlayerId) -> u32 {
    let mut result: u32 = 1;
    let mut additional: u32 = 0;
    for host in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in &host.static_abilities {
            if !st_ab.check_conditions_full(&StaticMode::Activations, host, game) {
                continue;
            }
            if !is_valid(st_ab, host, card, sa, activator, game) {
                continue;
            }
            let static_sa = SpellAbility::new_simple(Some(host.id), host.controller, "");
            if let Some(min_limit) = st_ab.ir.min_limit_text.as_deref() {
                let min = crate::svar::resolve_numeric_value(game, &static_sa, min_limit, 0);
                if min == -1 {
                    return u32::MAX;
                }
                result = result.max(min.max(0) as u32);
            }
            if let Some(amount) = st_ab.ir.additional_text.as_deref() {
                additional +=
                    crate::svar::resolve_numeric_value(game, &static_sa, amount, 0).max(0) as u32;
            }
        }
    }
    result + additional
}

fn is_valid(
    st_ab: &StaticAbility,
    host: &Card,
    card: &Card,
    sa: &SpellAbility,
    activator: PlayerId,
    game: &GameState,
) -> bool {
    valid_filter::matches_valid_card_selector_opt_in_game(
        st_ab.ir.valid_card.as_ref(),
        card,
        host,
        game,
    ) && st_ab
        .ir
        .valid_sa
        .as_deref()
        .is_none_or(|filter| crate::spellability::matches_valid_sa(filter, sa, host, Some(card)))
        && valid_filter::matches_valid_player_selector_opt(
            st_ab.ir.valid_player.as_ref(),
            activator,
            host.controller,
        )
}
