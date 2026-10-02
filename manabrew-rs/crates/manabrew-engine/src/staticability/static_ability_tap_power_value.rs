use std::sync::Arc;

use crate::card::{valid_filter, Card};
use crate::game::GameState;
use crate::spellability::{matches_valid_sa, SpellAbility};
use crate::staticability::StaticMode;

/// Check if a card should use toughness as its tap power value.
pub fn with_toughness(game: &GameState, card: &Card, sa: Option<&SpellAbility>) -> bool {
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|s| s.check_mode(&StaticMode::TapPowerValue) && s.zones_check(source.zone))
        {
            match st_ab.ir.value_text.as_deref() {
                Some(val) if val.eq_ignore_ascii_case("Toughness") => {}
                _ => continue,
            }

            // ValidCard$
            if !valid_filter::matches_valid_card_selector_opt_in_game(
                st_ab.ir.valid_card.as_ref(),
                card,
                source,
                game,
            ) {
                continue;
            }

            if let Some(valid_sa) = st_ab.ir.valid_sa.as_deref() {
                let Some(sa) = sa else {
                    continue;
                };
                if !matches_valid_sa(
                    valid_sa,
                    sa,
                    ability_host(&game.cards, sa),
                    crate::card::valid_filter::MatchContext::new(source, game),
                ) {
                    continue;
                }
            }

            return true;
        }
    }
    false
}

/// Get the modifier for tap power value.
pub fn get_mod(game: &GameState, card: &Card, sa: Option<&SpellAbility>) -> i32 {
    let mut total = 0;
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in source
            .static_abilities
            .iter()
            .filter(|s| s.check_mode(&StaticMode::TapPowerValue) && s.zones_check(source.zone))
        {
            // ValidCard$
            if !valid_filter::matches_valid_card_selector_opt_in_game(
                st_ab.ir.valid_card.as_ref(),
                card,
                source,
                game,
            ) {
                continue;
            }

            if let Some(valid_sa) = st_ab.ir.valid_sa.as_deref() {
                let Some(sa) = sa else {
                    continue;
                };
                if !matches_valid_sa(
                    valid_sa,
                    sa,
                    ability_host(&game.cards, sa),
                    crate::card::valid_filter::MatchContext::new(source, game),
                ) {
                    continue;
                }
            }

            if let Some(val) = st_ab.ir.value_text.as_deref() {
                total += val.parse::<i32>().unwrap_or(0);
            }
        }
    }
    total
}

fn ability_host<'a>(cards: &'a [Arc<Card>], sa: &SpellAbility) -> Option<&'a Card> {
    let source = sa.source?;
    cards.get(source.index()).map(Arc::as_ref)
}
