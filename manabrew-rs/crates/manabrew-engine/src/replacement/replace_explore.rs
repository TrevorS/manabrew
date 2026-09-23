//! Replacement logic for `Event$ Explore`.
//!
//! Mirrors Java `ReplaceExplore.java` in `forge/game/replacement/`.

use crate::agent::PlayerAgent;
use crate::card::Card;
use crate::game::GameState;
use crate::ids::CardId;

use super::replacement_effect::ReplacementEffect;
use super::replacement_handler::{ReplacementEvent, ReplacementRuntime};
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceExplore.canReplace()`.
/// Java checks `ValidExplorer` using the same pattern as `ValidCard`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::Explore {
        return false;
    }
    let card = match event {
        ReplacementEvent::Explore { card } => *card,
        _ => return false,
    };
    let target_card = &game.cards[card.index()];
    // Java uses ValidExplorer with the same semantics as ValidCard.
    if let Some(valid) = effect.ir.valid_explorer_text.as_deref() {
        if !effect.matches_valid_card(valid, target_card, source_card) {
            return false;
        }
    } else if let Some(valid) = effect.ir.valid_card_selector.as_ref() {
        if !effect.matches_compiled_valid_card(valid, target_card, source_card) {
            return false;
        }
    }
    true
}

/// Mirrors Java `ReplacementHandler.executeReplacement()` for Explore.
pub fn execute(
    effect: &ReplacementEffect,
    event: &mut ReplacementEvent,
    game: &mut GameState,
    source_card_id: CardId,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    runtime: Option<&mut ReplacementRuntime<'_>>,
) -> ReplacementResult {
    if effect.prevents() || effect.has_skip() {
        return ReplacementResult::Skipped;
    }
    if let Some(replace_with) = effect.replace_with() {
        super::replace_moved::execute_replace_with(
            effect,
            replace_with,
            game,
            source_card_id,
            event,
            agents,
            runtime,
        );
    }
    ReplacementResult::Replaced
}
