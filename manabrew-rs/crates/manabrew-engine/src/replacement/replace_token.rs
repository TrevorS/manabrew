//! Replacement logic for `Event$ CreateToken`.
//!
//! Mirrors Java `ReplaceToken.java` in `forge/game/replacement/`.

use crate::ability::effects::token_effect_base::TokenCreateTable;
use crate::ability::effects::{replace_token_effect, EffectContext};
use crate::agent::PlayerAgent;
use crate::card::Card;
use crate::game::GameState;
use crate::ids::CardId;
use crate::spellability::build_spell_ability;

use super::replacement_effect::ReplacementEffect;
use super::replacement_handler::{ReplacementEvent, ReplacementRuntime};
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceToken.filterAmount()`.
pub fn filter_amount(effect: &ReplacementEffect, table: &TokenCreateTable, host: &Card) -> usize {
    table.get_filter_amount(
        effect.base().get_param("ValidPlayer"),
        effect.base().get_param("ValidToken"),
        effect,
        host,
    )
}

/// Mirrors Java `ReplaceToken.canReplace()`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    _game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::CreateToken {
        return false;
    }
    let (player, token_table, is_effect) = match event {
        ReplacementEvent::CreateToken {
            player,
            token_table,
            is_effect,
        } => (*player, token_table, *is_effect),
        _ => return false,
    };
    // EffectOnly$ True: only apply to tokens created by effects, not game rules
    if effect.ir.effect_only && !is_effect {
        return false;
    }
    if let Some(valid) = effect.ir.valid_player_selector.as_ref() {
        if !effect.matches_compiled_valid_player(valid, player, source_card) {
            return false;
        }
    }
    filter_amount(effect, token_table, source_card) > 0
}

/// Mirrors Java `ReplacementHandler.executeReplacement()` for CreateToken.
pub fn execute(
    effect: &ReplacementEffect,
    event: &mut ReplacementEvent,
    game: &mut GameState,
    source_card_id: CardId,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    runtime: Option<&mut ReplacementRuntime<'_>>,
) -> ReplacementResult {
    let ReplacementEvent::CreateToken {
        player,
        token_table,
        ..
    } = event
    else {
        return ReplacementResult::NotReplaced;
    };
    let Some(raw) = effect.replace_with().and_then(|name| {
        crate::core::HasSVars::get_svar(&effect.base.card_trait_base, name)
            .map(str::to_string)
            .or_else(|| game.card(source_card_id).svars.get(name).cloned())
    }) else {
        return ReplacementResult::Replaced;
    };
    if crate::parsing::raw_get(&raw, "DB") != Some("ReplaceToken") {
        return ReplacementResult::Replaced;
    }
    let (Some(agents), Some(runtime)) = (agents, runtime) else {
        return ReplacementResult::NotReplaced;
    };
    let controller = game.card(source_card_id).controller;
    let sa = build_spell_ability(game, source_card_id, &raw, controller);
    let mut ctx = EffectContext {
        game,
        combat: None,
        agents,
        trigger_handler: runtime.trigger_handler,
        token_templates: runtime.token_templates,
        token_art_variants: runtime.token_art_variants,
        token_fallback: runtime.token_fallback,
        edition_dates: runtime.edition_dates,
        mana_pools: runtime.mana_pools,
        parent_target_card: None,
        rng: runtime.rng,
    };
    replace_token_effect::resolve_token_table(
        &mut ctx,
        &sa,
        effect,
        source_card_id,
        *player,
        token_table,
    );
    ReplacementResult::Updated
}
