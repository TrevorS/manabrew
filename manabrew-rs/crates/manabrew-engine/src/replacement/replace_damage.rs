//! Replacement logic for `Event$ DamageDone`.
//!
//! Mirrors Java `ReplaceDamage.java` in `forge/game/replacement/`.

use crate::ability::ability_utils::{get_defined_cards, get_defined_players};
use crate::card::Card;
use crate::game::GameState;
use crate::ids::CardId;
use crate::parsing::compare::compare_expr;

use super::replacement_effect::{resolve_replace_with_chain, ReplacementEffect};
use super::replacement_handler::ReplacementEvent;
use super::replacement_handler::{execute_replace_effect_ir, resolve_replace_value};
use super::replacement_result::ReplacementResult;
use super::replacement_type::ReplacementType;
use crate::card_trait_base::CardTrait;

/// Mirrors Java `ReplaceDamage.canReplace()`.
pub fn can_replace(
    effect: &ReplacementEffect,
    event: &ReplacementEvent,
    game: &GameState,
    source_card: &Card,
) -> bool {
    if effect.event != ReplacementType::DamageDone {
        return false;
    }
    let (damage_source, target_player, target_card, amount, is_combat) = match event {
        ReplacementEvent::DamageToCard {
            target,
            amount,
            source,
            is_combat,
        } => (*source, None, Some(*target), *amount, *is_combat),
        ReplacementEvent::DamageToPlayer {
            target,
            amount,
            source,
            is_combat,
        } => (*source, Some(*target), None, *amount, *is_combat),
        _ => return false,
    };
    if amount <= 0 {
        return false;
    }
    if let Some(valid_source) = effect.ir.valid_source_selector.as_ref() {
        let Some(source_id) = damage_source else {
            return false;
        };
        if !effect.matches_compiled_valid_card(valid_source, game.card(source_id), source_card) {
            return false;
        }
    }
    if let Some(valid_target) = effect.ir.valid_target_selector.as_ref() {
        let target_matches = if let Some(target) = target_player {
            crate::player::player_property::is_valid(
                target,
                valid_target,
                game,
                source_card.id,
                source_card.controller,
                &crate::spellability::SpellAbility::new_empty(
                    Some(source_card.id),
                    source_card.controller,
                ),
            )
        } else if let Some(target) = target_card {
            effect.matches_compiled_valid_card(valid_target, game.card(target), source_card)
        } else {
            false
        };
        if !target_matches {
            return false;
        }
    }
    if let Some(wants_max_speed) = effect.ir.max_speed {
        if wants_max_speed != (game.player(source_card.controller).speed == 4) {
            return false;
        }
    }
    if let Some(wants_combat) = effect.ir.is_combat {
        if wants_combat != is_combat {
            return false;
        }
    }
    if let Some(damage_amount) = effect.ir.damage_amount_text.as_deref() {
        let threshold = damage_amount.get(2..).unwrap_or("");
        let rhs = resolve_replace_value(threshold, game, source_card.id, event)
            .or_else(|| threshold.parse::<i32>().ok())
            .unwrap_or(0);
        let cmp = format!("{}{}", damage_amount.get(..2).unwrap_or("GE"), rhs);
        if !compare_expr(amount, &cmp) {
            return false;
        }
    }
    if let Some(def) = effect.ir.damage_target_text.as_deref() {
        let cant_be_redirected = "Damage that would be dealt to CARDNAME can't be redirected.";
        let affected_cant_be_redirected = match (target_player, target_card) {
            (Some(player), _) => crate::player::has_keyword(game, player, cant_be_redirected),
            (_, Some(card)) => game.card(card).has_keyword(cant_be_redirected),
            _ => false,
        };
        if affected_cant_be_redirected {
            return false;
        }
        if def.starts_with("Replaced") {
            if def == "ReplacedSourceController" {
                let Some(source_id) = damage_source else {
                    return false;
                };
                if game.player(game.card(source_id).controller).left_game {
                    return false;
                }
            } else if def == "ReplacedTargetController" {
                let Some(card) = target_card else {
                    return false;
                };
                if game.player(game.card(card).controller).left_game {
                    return false;
                }
            } else {
                return false;
            }
        } else {
            let controller = Some(source_card.controller);
            if get_defined_players(game, Some(source_card.id), def, controller)
                .into_iter()
                .any(|player| game.player(player).left_game)
            {
                return false;
            }
            if get_defined_cards(game, Some(source_card.id), def, controller)
                .into_iter()
                .any(|card| !game.card(card).can_be_dealt_damage())
            {
                return false;
            }
        }
    }
    true
}

/// Mirrors Java `ReplacementHandler.executeReplacement()` for DamageDone.
pub fn execute(
    effect: &ReplacementEffect,
    event: &mut ReplacementEvent,
    game: &GameState,
    source_card_id: CardId,
) -> ReplacementResult {
    match event {
        ReplacementEvent::DamageToCard { .. } | ReplacementEvent::DamageToPlayer { .. } => {}
        _ => return ReplacementResult::NotReplaced,
    }
    if effect.prevents() {
        match event {
            ReplacementEvent::DamageToCard { amount, .. } => *amount = 0,
            ReplacementEvent::DamageToPlayer { amount, .. } => *amount = 0,
            _ => {}
        }
        return ReplacementResult::Prevented;
    }
    // Handle built-in replacement modes before SVar chain.
    if let Some(replace) = effect.replace_with() {
        match replace {
            "DmgTwice" | "DoubleDamage" => {
                match event {
                    ReplacementEvent::DamageToCard { amount, .. } => *amount *= 2,
                    ReplacementEvent::DamageToPlayer { amount, .. } => *amount *= 2,
                    _ => {}
                }
                return ReplacementResult::Updated;
            }
            "DmgHalf" | "HalfDamage" => {
                match event {
                    ReplacementEvent::DamageToCard { amount, .. } => *amount = (*amount + 1) / 2,
                    ReplacementEvent::DamageToPlayer { amount, .. } => *amount = (*amount + 1) / 2,
                    _ => {}
                }
                return ReplacementResult::Updated;
            }
            "DmgPlus1" => {
                match event {
                    ReplacementEvent::DamageToCard { amount, .. } => *amount += 1,
                    ReplacementEvent::DamageToPlayer { amount, .. } => *amount += 1,
                    _ => {}
                }
                return ReplacementResult::Updated;
            }
            "DmgPlus2" => {
                match event {
                    ReplacementEvent::DamageToCard { amount, .. } => *amount += 2,
                    ReplacementEvent::DamageToPlayer { amount, .. } => *amount += 2,
                    _ => {}
                }
                return ReplacementResult::Updated;
            }
            _ => {}
        }
    }
    let chain_result = resolve_replace_with_chain(effect, game.card(source_card_id))
        .and_then(|chain| execute_replace_effect_ir(&chain, event, game, source_card_id, None));
    match effect.base.card_trait_base.get_param("ReplacementResult") {
        Some("Updated") => ReplacementResult::Updated,
        Some("NotReplaced") => ReplacementResult::NotReplaced,
        Some("Prevented") => ReplacementResult::Prevented,
        Some("Skipped") => ReplacementResult::Skipped,
        Some("Replaced") => ReplacementResult::Replaced,
        _ => chain_result.unwrap_or(ReplacementResult::Replaced),
    }
}
