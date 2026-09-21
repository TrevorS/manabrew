use forge_foundation::ZoneType;

use super::{matches_valid_cards_for_sa, EffectContext};
use crate::spellability::{build_spell_ability, SpellAbility};

/// `SP$ RepeatEach` — loop a sub-ability over cards or players.
///
/// Mirrors Java's `RepeatEachEffect.java`.
///
/// # Params
/// - `RepeatSubAbility` — SVar name on source card for the sub-ability to resolve each iteration
/// - `RepeatCards` / `DefinedCards` — iterate over matching or defined cards
/// - `RepeatPlayers` — iterate over the defined players
/// - `Zone` — zone to search for RepeatCards (default Battlefield)
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `RepeatEachEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(RepeatEachEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let source_id = match sa.source {
        Some(id) => id,
        None => return,
    };

    let controller = sa.activating_player;
    let use_damage_map = sa.ir.damage_map;
    let use_change_zone_table = sa.ir.change_zone_table;

    if use_damage_map {
        ctx.game.ensure_pending_damage_maps();
    }
    if use_change_zone_table {
        ctx.game.ensure_pending_change_zone_table();
    }

    // Get the sub-ability SVar name
    let sub_svar_name = match sa.ir.repeat_sub_ability.as_deref() {
        Some(name) => name,
        None => return,
    };

    // Look up the sub-ability text from the source card's SVars
    let sub_text = match ctx
        .game
        .card(source_id)
        .get_s_var(sub_svar_name)
        .map(str::to_string)
    {
        Some(text) => text,
        None => return,
    };

    let raw = sa.ability_text.as_str();
    let mut repeat_cards: Vec<crate::ids::CardId> = Vec::new();
    if let Some(repeat_cards_filter) = sa.ir.repeat_cards_text.as_deref() {
        let repeat_cards_selector = sa.ir.repeat_cards_selector.as_ref();
        let zone = sa.ir.zone.unwrap_or(ZoneType::Battlefield);
        for &pid in &ctx.game.player_order.clone() {
            for cid in ctx.game.cards_in_zone(zone, pid).to_vec() {
                if matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    repeat_cards_selector,
                    repeat_cards_filter,
                ) {
                    repeat_cards.push(cid);
                }
            }
        }
    } else if let Some(defined) = crate::parsing::raw_get(raw, "DefinedCards") {
        let expr = crate::ability::ability_ir::DefinedExpr::parse(defined);
        for d in &expr.refs {
            repeat_cards.extend(
                crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game,
                    sa,
                    d.as_legacy_str(),
                ),
            );
        }
    }

    if crate::parsing::raw_has_key(raw, "ClearRemembered") {
        ctx.game.card_mut(source_id).clear_remembered();
    }

    if !repeat_cards.is_empty() {
        if let Some(order) = crate::parsing::raw_get(raw, "ChooseOrder") {
            if repeat_cards.len() > 1 {
                let chooser = if order == "True" {
                    controller
                } else {
                    crate::ability::ability_utils::resolve_defined_players_with_sa(
                        order, sa, controller, ctx.game,
                    )
                    .first()
                    .copied()
                    .unwrap_or(controller)
                };
                ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
                let ordered = ctx.agents[chooser.index()].order_move_to_zone_list(
                    ctx.game,
                    chooser,
                    &repeat_cards,
                    ZoneType::None,
                );
                if ordered.len() == repeat_cards.len()
                    && repeat_cards.iter().all(|id| ordered.contains(id))
                {
                    repeat_cards = ordered;
                }
            }
        }
        let use_imprinted = crate::parsing::raw_has_key(raw, "UseImprinted");
        for card_id in repeat_cards {
            if use_imprinted {
                ctx.game.card_mut(source_id).add_imprinted_card(card_id);
            } else {
                ctx.game.card_mut(source_id).add_remembered_card(card_id);
            }
            let sub_sa = build_spell_ability(ctx.game, source_id, &sub_text, controller);
            resolve_sub_chain(ctx, sub_sa);
            if use_imprinted {
                ctx.game.card_mut(source_id).remove_imprinted_card(card_id);
            } else {
                ctx.game.card_mut(source_id).remove_remembered(card_id);
            }
            if ctx.game.game_over {
                break;
            }
        }
    }

    if let Some(def) = crate::parsing::raw_get(raw, "RepeatTypesFrom") {
        let cards =
            crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(ctx.game, sa, def);
        let mut valid_types: Vec<String> = Vec::new();
        for cid in cards {
            for ct in &ctx.game.card(cid).type_line.core_types {
                let name = ct.name().to_string();
                if !valid_types.contains(&name) {
                    valid_types.push(name);
                }
            }
        }
        let chooser = match crate::parsing::raw_get(raw, "ChooseOrder") {
            Some(order) if !order.eq_ignore_ascii_case("True") => {
                crate::ability::ability_utils::resolve_defined_players_with_sa(
                    order, sa, controller, ctx.game,
                )
                .first()
                .copied()
                .unwrap_or(controller)
            }
            _ => controller,
        };
        let stored_type = ctx.game.card(source_id).chosen_type.clone();
        while !valid_types.is_empty() {
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let Some(chosen) =
                ctx.agents[chooser.index()].choose_type(chooser, "Card", &valid_types)
            else {
                break;
            };
            ctx.game.card_mut(source_id).chosen_type = Some(chosen.clone());
            let sub_sa = build_spell_ability(ctx.game, source_id, &sub_text, controller);
            resolve_sub_chain(ctx, sub_sa);
            valid_types.retain(|t| t != &chosen);
            if ctx.game.game_over {
                break;
            }
        }
        ctx.game.card_mut(source_id).chosen_type = stored_type;
    }

    if let Some(repeat_players) = sa.ir.repeat_players.as_deref() {
        let mut players = Vec::new();
        for d in &crate::ability::ability_ir::DefinedExpr::parse(repeat_players).refs {
            for pid in crate::ability::ability_utils::resolve_defined_players_with_sa(
                d.as_legacy_str(),
                sa,
                controller,
                ctx.game,
            ) {
                if !players.contains(&pid) {
                    players.push(pid);
                }
            }
        }
        if crate::parsing::raw_has_key(raw, "ClearRememberedBeforeLoop") {
            ctx.game.card_mut(source_id).clear_remembered();
        }
        let optional = crate::parsing::raw_has_key(raw, "RepeatOptionalForEachPlayer");
        let message = crate::parsing::raw_get(raw, "RepeatOptionalMessage").unwrap_or_default();
        for pid in players {
            if optional {
                ctx.agents[pid.index()].snapshot_state(ctx.game, ctx.mana_pools);
                if !ctx.agents[pid.index()].confirm_action(
                    pid,
                    None,
                    message,
                    &[],
                    sa.source,
                    sa.api,
                ) {
                    continue;
                }
            }
            let temp_remembered =
                std::mem::take(&mut ctx.game.card_mut(source_id).remembered_players);
            ctx.game.card_mut(source_id).add_remembered_player(pid);

            // Java `RepeatEachEffect` keeps sa.getActivatingPlayer() across
            // iterations; pid flows in only via Remembered.
            let sub_sa = build_spell_ability(ctx.game, source_id, &sub_text, controller);
            resolve_sub_chain(ctx, sub_sa);

            let host = ctx.game.card_mut(source_id);
            host.remembered_players.retain(|&p| p != pid);
            for p in temp_remembered {
                host.add_remembered_player(p);
            }
            if ctx.game.game_over {
                break;
            }
        }
    }

    if use_damage_map {
        // Mirror Java RepeatEach post-loop damage-map resolve.
        let mut flush_sa = sa.clone();
        flush_sa.damage_map = ctx.game.pending_damage_map.clone();
        flush_sa.prevent_map = ctx.game.pending_prevent_map.clone();
        super::damage_resolve_effect::DamageResolveEffect::resolve(ctx, &flush_sa);
        ctx.game.clear_pending_damage_maps();
    }
    if use_change_zone_table {
        if let Some(table) = ctx.game.pending_change_zone_table.clone() {
            table.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
            ctx.game.clear_pending_change_zone_table();
        }
    }
}

/// Walk a sub-ability chain (same pattern as charm_effect.rs).
fn resolve_sub_chain(ctx: &mut EffectContext, initial: SpellAbility) {
    let mut cur_opt: Option<SpellAbility> = Some(initial);
    while let Some(cur_sa) = cur_opt {
        super::resolve_effect(ctx, &cur_sa);
        cur_opt = cur_sa.sub_ability.map(|b| *b);
        if ctx.game.game_over {
            break;
        }
    }
}
