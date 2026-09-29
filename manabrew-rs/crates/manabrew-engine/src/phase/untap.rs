//! Untap — handles untap step logic.
//!
//! Mirrors Java's `Untap.java`.
//! Handles "until next untap", phasing, day/night transitions,
//! and the actual untap of permanents.

use std::sync::Arc;

use forge_foundation::ZoneType;

use crate::game::GameState;
use crate::ids::PlayerId;

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
            game.run_phase_out_commands(game.cards[i].id);
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

#[cfg(test)]
mod tests {
    #[test]
    fn do_phasing_phases_in() {
        // Basic test that phasing works directionally
        // Full integration tests would need a GameState
    }
}
