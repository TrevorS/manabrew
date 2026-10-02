//! AbilityStatic -- helper functions for static abilities (e.g. morph face-up).
//! Mirrors Java's `AbilityStatic.java`.
//! Static abilities are special actions that don't use the stack (like turning
//! a morph face-up).

use crate::game::GameState;
use crate::replacement::replacement_handler::{cant_happen_check, ReplacementEvent};
use crate::spellability::SpellAbility;

/// Type alias for SpellAbility when used as a static ability.
/// In Java, `AbilityStatic` is a subclass; in Rust it's the same struct.
pub type AbilityStatic = super::SpellAbility;

/// Mirrors Java's `AbilityStatic.canPlay()`: no split second, suppression, detention or
/// `CantBeActivated` check, which belong to `AbilityActivated`. The face-down test stands in
/// for the `IsPresent$ Card.Self+faceDown` Java writes into the turn-face-up ability.
pub fn can_play(sa: &SpellAbility, game: &GameState) -> bool {
    let Some(card_id) = sa.source else {
        return false;
    };
    if sa.is_turn_face_up()
        && (!game.card(card_id).face_down
            || cant_happen_check(game, &ReplacementEvent::TurnFaceUp { card: card_id }))
    {
        return false;
    }
    sa.can_play(game)
}
