//! Replacement logic for `Event$ Destroy`.
//!
//! Mirrors Java `ReplaceDestroy.java` in `forge/game/replacement/`.

use crate::card::Card;
use crate::game::GameState;
use crate::ids::CardId;

use super::replacement_effect::ReplacementEffect;
use super::replacement_handler::ReplacementEvent;
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceDestroy.canReplace()`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::Destroy {
        return false;
    }
    let ReplacementEvent::Destroy {
        target,
        cause,
        regeneration,
    } = event
    else {
        return false;
    };
    let target_card = &game.cards[target.index()];
    if let Some(valid) = effect.ir.valid_card_selector.as_ref() {
        if !effect.matches_compiled_valid_card(valid, target_card, source_card, game) {
            return false;
        }
    }
    if effect.base.card_trait_base.has_param("Regeneration") {
        if !*regeneration
            || crate::staticability::static_ability_cant_regenerate::cant_regenerate(
                game,
                target_card,
            )
        {
            return false;
        }
        if target_card.is_creature() && target_card.toughness() <= 0 {
            return false;
        }
    }
    if let Some(valid) = effect.ir.valid_cause_text.as_deref() {
        let Some(cause) = cause else {
            return false;
        };
        let ability_host = cause.source.map(|card| game.card(card));
        if !crate::spellability::matches_valid_sa(
            valid,
            cause,
            ability_host,
            crate::card::valid_filter::MatchContext::new(source_card, game),
        ) {
            return false;
        }
    }
    true
}

pub fn execute(
    effect: &ReplacementEffect,
    event: &mut ReplacementEvent,
    game: &mut GameState,
    source_card_id: CardId,
    agents: Option<&mut [Box<dyn crate::agent::PlayerAgent>]>,
    runtime: Option<&mut super::replacement_handler::ReplacementRuntime<'_>>,
) -> ReplacementResult {
    if let Some(replace_with) = effect.replace_with() {
        if !super::replace_moved::execute_replace_with(
            effect,
            replace_with,
            game,
            source_card_id,
            event,
            agents,
            runtime,
        ) {
            return ReplacementResult::NotReplaced;
        }
    } else if let Some(ability) = effect.base.get_overriding_ability() {
        if !super::replace_moved::execute_replacement_ability(
            effect,
            ability.clone(),
            game,
            event,
            agents,
            runtime,
        ) {
            return ReplacementResult::NotReplaced;
        }
    }
    ReplacementResult::Replaced
}
