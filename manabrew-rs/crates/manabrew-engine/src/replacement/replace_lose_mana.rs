//! Replacement logic for `Event$ LoseMana`.
//!
//! Mirrors Java `ReplaceLoseMana.java` in `forge/game/replacement/`.

use crate::card::Card;
use crate::core::HasSVars;
use crate::game::GameState;
use crate::ids::CardId;
use crate::parsing::{keys, Params};
use forge_foundation::ManaAtom;

use super::replacement_effect::ReplacementEffect;
use super::replacement_handler::ReplacementEvent;
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceLoseMana.canReplace()`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    _game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::LoseMana {
        return false;
    }
    let player = match event {
        ReplacementEvent::LoseMana { player, .. } => *player,
        _ => return false,
    };
    if let Some(valid) = effect.ir.valid_player_selector.as_ref() {
        if !effect.matches_compiled_valid_player(valid, player, source_card) {
            return false;
        }
    }
    true
}

/// Mirrors Java `ReplacementHandler.executeReplacement()` for LoseMana.
pub fn execute(
    effect: &ReplacementEffect,
    event: &mut ReplacementEvent,
    game: &GameState,
    source_card_id: CardId,
) -> ReplacementResult {
    if effect.prevents() || effect.has_skip() {
        return ReplacementResult::Skipped;
    }
    let replace_type = effect
        .replace_with()
        .and_then(|name| game.card(source_card_id).get_svar(name))
        .map(Params::from_raw)
        .and_then(|params| params.get(keys::REPLACE_TYPE).map(str::to_ascii_lowercase));
    if let (Some(replace_type), ReplacementEvent::LoseMana { mana, .. }) = (replace_type, event) {
        *mana = ManaAtom::from_name(&replace_type);
    }
    ReplacementResult::Replaced
}
