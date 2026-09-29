//! Replacement logic for `Event$ Counter` (countering a spell).
//!
//! Mirrors Java `ReplaceCounter.java` in `forge/game/replacement/`.

use crate::card::Card;
use crate::game::GameState;
use crate::ids::CardId;

use super::replacement_effect::ReplacementEffect;
use super::replacement_handler::ReplacementEvent;
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceCounter.canReplace()`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::Counter {
        return false;
    }
    let ReplacementEvent::Counter {
        card: target_id,
        spell_ability,
        cause,
    } = event
    else {
        return false;
    };
    let target_card = &game.cards[target_id.index()];
    if let Some(valid) = effect.ir.valid_card_selector.as_ref() {
        if !effect.matches_compiled_valid_card(valid, target_card, source_card) {
            return false;
        }
    }
    [
        (effect.ir.valid_sa_text.as_deref(), spell_ability),
        (effect.ir.valid_cause_text.as_deref(), cause),
    ]
    .into_iter()
    .all(|(valid, sa)| {
        valid.is_none_or(|valid| {
            crate::spellability::matches_valid_sa(
                valid,
                sa,
                source_card,
                sa.source.map(|id| game.card(id)),
            )
        })
    })
}

/// CantHappen layer prevents countering (e.g. "can't be countered").
pub fn execute(
    _effect: &ReplacementEffect,
    _event: &mut ReplacementEvent,
    _game: &GameState,
    _source_card_id: CardId,
) -> ReplacementResult {
    ReplacementResult::Replaced
}
