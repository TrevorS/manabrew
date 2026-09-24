//! ReplaceToken effect — replace token creation with another effect.
//!
//! Ported from Java's `ReplaceTokenEffect.java`.

use forge_foundation::ZoneType;

use super::copy_permanent_effect;
use super::token_effect_base::{TokenCreateTable, TokenEffectBase, TOKEN_EFFECT_BASE};
use super::EffectContext;
use crate::ability::ability_utils::matches_valid_cards_for_sa;
use crate::agent::types::GameEntity;
use crate::card_trait_base::{CardTrait, MatchValidTarget};
use crate::ids::{CardId, PlayerId};
use crate::parsing::raw_get;
use crate::player::player_controller::PlayerController;
use crate::replacement::replacement_effect::ReplacementEffect;
use crate::spellability::SpellAbility;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ReplaceTokenEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ReplaceTokenEffect)]
fn resolve(_ctx: &mut EffectContext, _sa: &crate::spellability::SpellAbility) {
    // Token replacement is handled by the replacement handler system.
}

pub fn resolve_token_table(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    re: &ReplacementEffect,
    host_id: CardId,
    affected: PlayerId,
    table: &mut TokenCreateTable,
) {
    let p = sa.activating_player;
    let host = ctx.game.card(host_id).clone();
    let valid_token = |prototype: &crate::card::Card| {
        re.matches_valid_param(
            "ValidToken",
            &MatchValidTarget::Card(prototype),
            Some(&host),
        )
    };

    match raw_get(&sa.ability_text, "Type") {
        Some("Amount") => {
            let modifier = raw_get(&sa.ability_text, "Amount").unwrap_or("Twice");
            for cell in table.cells_mut() {
                if cell.owner != affected || !valid_token(&cell.prototype) {
                    continue;
                }
                cell.amount =
                    crate::svar::do_x_math(cell.amount as i32, modifier, ctx.game, host_id, p, sa)
                        .max(0) as usize;
            }
        }
        Some("AddToken") => {
            let mut by_controller: Vec<(PlayerId, usize)> = Vec::new();
            for cell in table.cells() {
                if cell.owner != affected || !valid_token(&cell.prototype) {
                    continue;
                }
                let controller = cell.prototype.controller;
                match by_controller.iter_mut().find(|(c, _)| *c == controller) {
                    Some((_, amount)) => *amount += cell.amount,
                    None => by_controller.push((controller, cell.amount)),
                }
            }
            if by_controller.is_empty() {
                return;
            }
            if let Some(amount) = raw_get(&sa.ability_text, "Amount") {
                let i = crate::svar::resolve_numeric_value(ctx.game, sa, amount, 0).max(0) as usize;
                for (_, value) in &mut by_controller {
                    *value = i;
                }
            }
            for (controller, amount) in by_controller {
                for script in TOKEN_EFFECT_BASE.token_scripts(sa) {
                    let mut token = TOKEN_EFFECT_BASE.get_proto_type(ctx, &script, sa, p);
                    token.set_code = Some(ctx.sync_token_art_rng(&script, sa));
                    token.set_controller(controller);
                    table.put(p, token, amount);
                }
            }
        }
        Some("ReplaceToken") => {
            let mut chosen = None;
            if let Some(valid_choices) = raw_get(&sa.ability_text, "ValidChoices") {
                let choices: Vec<GameEntity> = ctx
                    .game
                    .players
                    .iter()
                    .flat_map(|player| {
                        ctx.game
                            .cards_in_zone(ZoneType::Battlefield, player.id)
                            .iter()
                            .copied()
                    })
                    .filter(|&cid| {
                        matches_valid_cards_for_sa(
                            ctx.game,
                            sa,
                            ctx.game.card(cid),
                            None,
                            valid_choices,
                        )
                    })
                    .map(GameEntity::Card)
                    .collect();
                if choices.is_empty() {
                    return;
                }
                let agent = ctx.agents[p.index()].as_mut();
                let mut controller = PlayerController::new(ctx.game, p, agent);
                controller.snapshot_state(ctx.mana_pools);
                chosen = match controller.choose_single_entity_for_effect(&choices) {
                    Some(GameEntity::Card(card_id)) => Some(card_id),
                    _ => None,
                };
            }

            let mut to_insert = Vec::new();
            table.cells_mut().retain(|cell| {
                if cell.owner != affected || !valid_token(&cell.prototype) {
                    return true;
                }
                to_insert.push((
                    cell.prototype.controller,
                    cell.amount,
                    cell.prototype.remembered_cards.clone(),
                ));
                false
            });

            for (controller, amount, remembered) in to_insert {
                if amount == 0 {
                    continue;
                }
                for script in TOKEN_EFFECT_BASE.token_scripts(sa) {
                    let mut token = if script == "Chosen" {
                        let Some(chosen) = chosen else {
                            continue;
                        };
                        let mut copy = copy_permanent_effect::get_proto_type(
                            sa,
                            ctx.game.card(chosen),
                            controller,
                        );
                        copy.copied_permanent = Some(chosen);
                        if copy.get_s_var("TokenScript").is_some() {
                            ctx.rng.next_int(1);
                        }
                        copy
                    } else {
                        let mut token =
                            TOKEN_EFFECT_BASE.get_proto_type(ctx, &script, sa, controller);
                        token.set_code = Some(ctx.sync_token_art_rng(&script, sa));
                        token
                    };
                    token.set_controller(controller);
                    token.add_remembered_cards(remembered.clone());
                    table.put(affected, token, amount);
                }
            }
        }
        Some("ReplaceController") => {
            let new_controller = raw_get(&sa.ability_text, "NewController")
                .and_then(|defined| {
                    crate::ability::ability_utils::resolve_defined_players_with_sa(
                        defined, sa, p, ctx.game,
                    )
                    .into_iter()
                    .next()
                })
                .unwrap_or(p);
            for cell in table.cells_mut() {
                if cell.owner != affected || !valid_token(&cell.prototype) {
                    continue;
                }
                cell.prototype.set_controller(new_controller);
            }
        }
        _ => {}
    }
}
