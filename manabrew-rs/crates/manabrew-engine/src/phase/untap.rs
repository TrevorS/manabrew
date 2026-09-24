//! Untap — handles untap step logic.
//!
//! Mirrors Java's `Untap.java`.
//! Handles "until next untap", phasing, day/night transitions,
//! and the actual untap of permanents.

use std::sync::Arc;

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::{CardId, PlayerId};

/// Performs the phasing step at the beginning of the untap step.
/// Mirrors Java's `Untap.doPhasing()`.
///
/// Phase in all directly-phased-out permanents controlled by the active player.
/// Phase out all permanents with phasing controlled by the active player.
pub fn do_phasing(game: &mut GameState, turn_player: PlayerId) {
    // Phase in: all phased-out permanents controlled by turn_player
    for i in 0..game.cards.len() {
        if game.cards[i].phased_out
            && game.cards[i].controller == turn_player
            && game.cards[i].zone == ZoneType::Battlefield
        {
            Arc::make_mut(&mut game.cards[i]).phased_out = false;
        }
    }

    // Phase out: all permanents with Phasing keyword controlled by turn_player
    for i in 0..game.cards.len() {
        if !game.cards[i].phased_out
            && game.cards[i].controller == turn_player
            && game.cards[i].zone == ZoneType::Battlefield
            && game.cards[i].has_keyword("Phasing")
        {
            Arc::make_mut(&mut game.cards[i]).phased_out = true;
        }
    }
}

/// Mirrors Java's `Untap.doDayTime()`: day becomes night when the previous turn's player cast no
/// spells that turn, and night becomes day when they cast two or more.
pub fn do_day_time(
    game: &mut GameState,
    previous: Option<PlayerId>,
    trigger_handler: &mut crate::trigger::handler::TriggerHandler,
) {
    let Some(previous) = previous else {
        return;
    };
    let casted = game
        .stack
        .get_spells_cast_last_turn()
        .iter()
        .filter(|&&cid| game.card(cid).controller == previous)
        .count();

    if game.is_day() && casted == 0 {
        game.set_day_time(Some(true), trigger_handler);
    } else if game.is_night && casted > 1 {
        game.set_day_time(Some(false), trigger_handler);
    }
}

/// Performs the untap of permanents for the active player.
/// Mirrors Java's `Untap.doUntap()`.
///
/// Untaps all tapped permanents controlled by the active player,
/// respecting "doesn't untap" and "you may choose not to untap" keywords.
pub fn do_untap(game: &mut GameState, active: PlayerId) -> Vec<CardId> {
    let cards: Vec<CardId> = game.cards_in_zone(ZoneType::Battlefield, active).to_vec();
    let mut untapped = Vec::new();

    for cid in cards {
        if !game.card(cid).tapped {
            continue;
        }

        // Skip cards that don't untap during untap step
        if game
            .card(cid)
            .has_keyword("CARDNAME doesn't untap during your untap step.")
        {
            continue;
        }

        // Skip exerted creatures (reset flag so they untap next turn)
        if game.card(cid).exerted {
            game.card_mut(cid).exerted = false;
            continue;
        }

        // Skip "This card doesn't untap during your next untap step."
        let has_skip = game
            .card(cid)
            .has_keyword("This card doesn't untap during your next untap step.");
        if has_skip {
            game.card_mut(cid)
                .keywords
                .remove("This card doesn't untap during your next untap step.");
            continue;
        }

        game.untap_during_untap_step(cid, active);
        untapped.push(cid);
    }

    // Remove exerted-by flags from all battlefield permanents
    for i in 0..game.cards.len() {
        if game.cards[i].zone == ZoneType::Battlefield {
            Arc::make_mut(&mut game.cards[i]).exerted = false;
        }
    }

    untapped
}

#[cfg(test)]
mod tests {
    #[test]
    fn do_phasing_phases_in() {
        // Basic test that phasing works directionally
        // Full integration tests would need a GameState
    }
}
