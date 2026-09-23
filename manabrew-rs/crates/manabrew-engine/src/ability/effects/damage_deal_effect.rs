use forge_foundation::ZoneType;

use super::{resolve_numeric_svar, EffectContext};
use crate::ability::ability_ir::EffectIr;
use crate::card::card_damage_map::DamageTarget;
use crate::card::card_util;
use crate::parsing::amount::AmountExpr;
use crate::parsing::keys;
use crate::spellability::SpellAbility;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DamageDealEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DamageDealEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let damage = resolve_damage_amount(ctx, sa);
    let use_damage_map = ctx.game.pending_damage_map.is_some() || sa.ir.damage_map;
    if sa.ir.damage_map {
        ctx.game.ensure_pending_damage_maps();
    }

    let sources: Vec<crate::ids::CardId> =
        match crate::parsing::raw_get(&sa.ability_text, "DamageSource") {
            Some(defined) => crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                ctx.game, sa, defined,
            ),
            None => sa.source.into_iter().collect(),
        };
    if sources.is_empty() {
        return;
    }

    let (target_players, mut target_cards) =
        crate::ability::spell_ability_effect::get_target_entities(ctx.game, sa);
    if let Some(decider) = sa.ir.optional_decider.as_deref().and_then(|decider| {
        crate::ability::ability_utils::resolve_defined_players_with_sa(
            decider,
            sa,
            sa.activating_player,
            ctx.game,
        )
        .into_iter()
        .next()
    }) {
        ctx.agents[decider.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[decider.index()].confirm_action(
            decider,
            None,
            &format!("Do you want to deal {damage} damage?"),
            &[],
            sa.source,
            Some(crate::ability::api_type::ApiType::DealDamage),
        ) {
            return;
        }
    }
    for cid in card_util::get_radiance(ctx.game, sa).iter().copied() {
        if !target_cards.contains(&cid) {
            target_cards.push(cid);
        }
    }

    let mut stored_excess = 0;
    for source in sources {
        stored_excess += deal_damage_from_source(
            ctx,
            sa,
            source,
            damage,
            use_damage_map,
            &target_players,
            &target_cards,
        );
    }
    if let (Some(excess_svar), Some(host)) = (
        crate::parsing::raw_get(&sa.ability_text, "ExcessSVar"),
        sa.source,
    ) {
        ctx.game
            .card_mut(host)
            .set_s_var(excess_svar, stored_excess.to_string());
    }

    let _ = crate::ability::spell_ability_effect::replace_dying(ctx.game, sa);
}

fn excess_damage_value(
    game: &crate::game::GameState,
    card_id: crate::ids::CardId,
    source: crate::ids::CardId,
) -> i32 {
    let card = game.card(card_id);
    if card.is_creature() && game.card(source).has_deathtouch() {
        return 1.min((card.toughness() - card.damage).max(0));
    }
    if card.is_creature() {
        return (card.toughness() - card.damage).max(0);
    }
    if card.type_line.is_planeswalker() {
        return card
            .counters
            .get(&crate::card::CounterType::Loyalty)
            .copied()
            .unwrap_or(0);
    }
    0
}

fn excess_svar_condition(
    game: &crate::game::GameState,
    sa: &SpellAbility,
    card_id: crate::ids::CardId,
) -> bool {
    match crate::parsing::raw_get(&sa.ability_text, "ExcessSVarCondition") {
        Some(valid) => super::matches_valid_cards_for_sa(game, sa, game.card(card_id), None, valid),
        None => true,
    }
}

#[allow(clippy::too_many_arguments)]
fn deal_damage_from_source(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    source: crate::ids::CardId,
    damage: i32,
    use_damage_map: bool,
    target_players: &[crate::ids::PlayerId],
    target_cards: &[crate::ids::CardId],
) -> i32 {
    let mut stored_excess = 0;
    let mut lifelink_dealt = 0;
    // Check source card for Infect/Wither keywords
    let (source_has_infect_keyword, source_has_wither) = if let Some(src_id) = Some(source) {
        let src = ctx.game.card(src_id);
        (
            src.has_infect(),
            src.has_wither()
                || crate::staticability::static_ability_wither_damage::is_wither_damage(
                    &ctx.game.cards,
                    src,
                ),
        )
    } else {
        (false, false)
    };

    // Overload: deal damage to ALL valid creatures instead of the chosen target.
    if sa.overloaded {
        let valid_tgts = sa
            .target_restrictions
            .as_ref()
            .and_then(|restrictions| restrictions.valid_tgts.first())
            .map(String::as_str)
            .unwrap_or_default();
        let valid_tgts_selector = sa
            .target_restrictions
            .as_ref()
            .map(|restrictions| &restrictions.valid_tgts_selector);
        let all_bf: Vec<crate::ids::CardId> = ctx
            .game
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| ctx.game.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();
        for cid in all_bf {
            if ctx.game.card(cid).zone != ZoneType::Battlefield {
                continue;
            }
            if !super::matches_valid_cards_for_sa(
                ctx.game,
                sa,
                ctx.game.card(cid),
                valid_tgts_selector,
                valid_tgts,
            ) {
                continue;
            }
            // Track damage source for DamagedBy trigger filters
            if let Some(src_id) = Some(source) {
                if !ctx
                    .game
                    .card(cid)
                    .damage_sources_this_turn
                    .contains(&src_id)
                {
                    ctx.game.card_mut(cid).add_damage_source_this_turn(src_id);
                }
            }
            if source_has_infect_keyword || source_has_wither {
                if use_damage_map {
                    if let Some(src_id) = Some(source) {
                        if let Some(map) = ctx.game.pending_damage_map.as_mut() {
                            map.put(src_id, DamageTarget::Card(cid), damage);
                        }
                    }
                } else if !crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
                    &ctx.game.cards,
                    ctx.game.card(cid),
                    &crate::card::CounterType::M1M1,
                ) {
                    crate::ability::effects::effect_context::add_counter_with_context(
                        ctx.game,
                        Some(ctx.trigger_handler),
                        Some(ctx.agents),
                        cid,
                        &crate::card::CounterType::M1M1,
                        damage,
                        crate::event::RunParams {
                            source_player: Some(source).map(|src_id| ctx.game.card(src_id).controller),
                            cause: Some(sa.clone()),
                            ..Default::default()
                        },
                        true,
                    );
                }
            } else if use_damage_map {
                if let Some(src_id) = Some(source) {
                    if let Some(map) = ctx.game.pending_damage_map.as_mut() {
                        map.put(src_id, DamageTarget::Card(cid), damage);
                    }
                }
            } else {
                ctx.game
                    .deal_damage_to_card_from(cid, damage, Some(source), false);
            }
            if !use_damage_map {
                ctx.trigger_handler.run_trigger(
                    crate::trigger::TriggerType::DamageDone,
                    crate::event::RunParams {
                        damage_source: Some(source),
                        damage_target_card: Some(cid),
                        damage_amount: Some(damage),
                        is_combat_damage: Some(false),
                        ..Default::default()
                    },
                    false,
                );
            }
        }
        return 0;
    }

    for &target_player in target_players {
        let source_has_infect = if let Some(src_id) = Some(source) {
            let src = ctx.game.card(src_id);
            source_has_infect_keyword
                || crate::staticability::static_ability_infect_damage::is_infect_damage(
                    ctx.game,
                    &ctx.game.cards,
                    target_player,
                    src.controller,
                )
        } else {
            false
        };
        if source_has_infect {
            // Infect: deal damage to players as poison counters
            if use_damage_map {
                if let Some(src_id) = Some(source) {
                    if let Some(map) = ctx.game.pending_damage_map.as_mut() {
                        map.put(src_id, DamageTarget::Player(target_player), damage);
                    }
                }
            } else {
                ctx.add_player_counter(
                    target_player,
                    &crate::card::CounterType::Poison,
                    damage,
                    sa,
                    crate::event::RunParams {
                        source_player: Some(source).map(|source| ctx.game.card(source).controller),
                        ..Default::default()
                    },
                );
            }
        } else if use_damage_map {
            if let Some(src_id) = Some(source) {
                if let Some(map) = ctx.game.pending_damage_map.as_mut() {
                    map.put(src_id, DamageTarget::Player(target_player), damage);
                }
            }
        } else {
            let dealt =
                ctx.game
                    .deal_damage_to_player_from(target_player, damage, Some(source), false);
            lifelink_dealt += dealt;
            if sa.ir.remember_damaged && dealt > 0 {
                ctx.game
                    .card_mut(source)
                    .add_remembered_player(target_player);
            }
            ctx.game.record_player_damage_assignment(
                Some(source),
                Some(target_player),
                dealt,
                false,
            );
        }

        // Record damage dealt by source for TotalDamageDoneByThisTurn SVar
        if !use_damage_map {
            if let Some(src_id) = Some(source) {
                if damage > 0 {
                    ctx.game.card_mut(src_id).total_damage_done_this_turn += damage;
                    ctx.game
                        .card_mut(src_id)
                        .damage_history
                        .record_damage(damage, false);
                }
            }
        }

        // Fire DamageDone trigger
        if !use_damage_map {
            ctx.trigger_handler.run_trigger(
                crate::trigger::TriggerType::DamageDone,
                crate::event::RunParams {
                    damage_source: Some(source),
                    damage_target_player: Some(target_player),
                    damage_amount: Some(damage),
                    is_combat_damage: Some(false),
                    ..Default::default()
                },
                false,
            );
            ctx.trigger_handler.run_trigger(
                crate::trigger::TriggerType::DamageDoneOnce,
                crate::event::RunParams {
                    damage_target_player: Some(target_player),
                    damage_amount: Some(damage),
                    is_combat_damage: Some(false),
                    ..Default::default()
                },
                false,
            );
            ctx.trigger_handler.flush_waiting_triggers(ctx.game);
        }
    }
    for &target_card in target_cards {
        if ctx.game.card(target_card).zone == ZoneType::Battlefield
            && !ctx.game.card(target_card).phased_out
        {
            // Protection: prevents all damage from matching sources
            if let Some(src_id) = Some(source) {
                if crate::staticability::static_ability_colorless_damage_source::target_is_protected_from_source(
                    &ctx.game.cards,
                    ctx.game.card(target_card),
                    ctx.game.card(src_id),
                ) {
                    continue;
                }
            }

            // Track damage source for DamagedBy trigger filters
            if let Some(src_id) = Some(source) {
                if !ctx
                    .game
                    .card(target_card)
                    .damage_sources_this_turn
                    .contains(&src_id)
                {
                    ctx.game
                        .card_mut(target_card)
                        .damage_sources_this_turn
                        .push(src_id);
                }
            }
            if source_has_infect_keyword || source_has_wither {
                // Infect/Wither: damage to creatures as -1/-1 counters
                if use_damage_map {
                    if let Some(src_id) = Some(source) {
                        if let Some(map) = ctx.game.pending_damage_map.as_mut() {
                            map.put(src_id, DamageTarget::Card(target_card), damage);
                        }
                    }
                } else if !crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
                    &ctx.game.cards,
                    ctx.game.card(target_card),
                    &crate::card::CounterType::M1M1,
                ) {
                    crate::ability::effects::effect_context::add_counter_with_context(
                        ctx.game,
                        Some(ctx.trigger_handler),
                        Some(ctx.agents),
                        target_card,
                        &crate::card::CounterType::M1M1,
                        damage,
                        crate::event::RunParams {
                            source_player: Some(source).map(|src_id| ctx.game.card(src_id).controller),
                            cause: Some(sa.clone()),
                            ..Default::default()
                        },
                        true,
                    );
                }
            } else if use_damage_map {
                if let Some(src_id) = Some(source) {
                    if let Some(map) = ctx.game.pending_damage_map.as_mut() {
                        map.put(src_id, DamageTarget::Card(target_card), damage);
                    }
                }
            } else {
                let lethal = excess_damage_value(ctx.game, target_card, source);
                let before = ctx.game.card(target_card).damage;
                ctx.game
                    .deal_damage_to_card_from(target_card, damage, Some(source), false);
                // What landed, not what was asked for: protection and prevention shields make
                // this smaller, and Java sums `addDamageAfterPrevention`'s return the same way.
                let landed = (ctx.game.card(target_card).damage - before).max(0);
                lifelink_dealt += landed;
                if sa.ir.remember_damaged && landed > 0 {
                    ctx.game.card_mut(source).add_remembered_card(target_card);
                }
                if damage > lethal && excess_svar_condition(ctx.game, sa, target_card) {
                    stored_excess += damage - lethal;
                }
            }

            // Record damage dealt by source for TotalDamageDoneByThisTurn SVar
            if !use_damage_map {
                if let Some(src_id) = Some(source) {
                    if damage > 0 {
                        ctx.game.card_mut(src_id).total_damage_done_this_turn += damage;
                        ctx.game
                            .card_mut(src_id)
                            .damage_history
                            .record_damage(damage, false);
                    }
                }
            }

            // Fire DamageDone trigger
            if !use_damage_map {
                ctx.trigger_handler.run_trigger(
                    crate::trigger::TriggerType::DamageDone,
                    crate::event::RunParams {
                        damage_source: Some(source),
                        damage_target_card: Some(target_card),
                        damage_amount: Some(damage),
                        is_combat_damage: Some(false),
                        ..Default::default()
                    },
                    false,
                );
                // Fire DamageDoneOnce batch trigger for non-map (non-combat)
                // damage.  Java fires this from CardDamageMap.triggerDamageOnce
                // which is called for ALL damage paths.  Without this, "when
                // dealt damage" triggers using DamageDoneOnce (e.g. Raptor
                // Hatchling Enrage) would never fire for spell damage.
                ctx.trigger_handler.run_trigger(
                    crate::trigger::TriggerType::DamageDoneOnce,
                    crate::event::RunParams {
                        damage_target_card: Some(target_card),
                        damage_amount: Some(damage),
                        is_combat_damage: Some(false),
                        ..Default::default()
                    },
                    false,
                );
                // Pre-match damage triggers while the creature is still on the
                // battlefield.  SBAs run after resolution and would move
                // lethally damaged creatures to the graveyard, causing their
                // Enrage triggers to fail the active-zone check.
                ctx.trigger_handler.flush_waiting_triggers(ctx.game);
            }

            if sa.ir.remember_damaged_creature {
                if let Some(src_id) = Some(source) {
                    let src = ctx.game.card_mut(src_id);
                    src.add_remembered_card(target_card);
                }
            }
        }
    }

    // CR 702.15e: one gain for the whole event. Mirrors the combat lifelink path in
    // `combat/mod.rs` — the can't-gain check and the GainLife replacement chain apply here too.
    if lifelink_dealt > 0 && ctx.game.card(source).has_lifelink() {
        let controller = ctx.game.card(source).controller;
        if !crate::staticability::static_ability_cant_gain_lose_pay_life::cant_gain_life(
            ctx.game, controller,
        ) {
            let mut gl_event =
                crate::replacement::replacement_handler::ReplacementEvent::GainLife {
                    player: controller,
                    amount: lifelink_dealt,
                };
            let gl_result = crate::replacement::replacement_handler::apply_replacements(
                ctx.game,
                &mut gl_event,
            );
            if gl_result != crate::replacement::ReplacementResult::Skipped
                && gl_result != crate::replacement::ReplacementResult::Replaced
            {
                let final_amount =
                    if let crate::replacement::replacement_handler::ReplacementEvent::GainLife {
                        amount,
                        ..
                    } = gl_event
                    {
                        amount
                    } else {
                        lifelink_dealt
                    };
                if final_amount > 0 {
                    ctx.game.player_gain_life(controller, final_amount);
                    ctx.game
                        .player_add_team_life_gained(controller, final_amount);
                }
            }
        }
    }

    stored_excess
}

/// Resolve the NumDmg$ parameter, supporting both integer literals and SVar
/// references (e.g. `NumDmg$ X` where `SVar:X:ParentTargeted$CardPower`).
/// Mirrors Java's `AbilityUtils.calculateAmount(sa, "NumDmg", sa)`.
fn resolve_damage_amount(ctx: &EffectContext, sa: &SpellAbility) -> i32 {
    if let Some(EffectIr::DealDamage(ir)) = &sa.ir.effect {
        if let Some(amount) = &ir.amount {
            if let Some(value) = resolve_amount_expr(ctx, sa, amount) {
                #[cfg(debug_assertions)]
                debug_assert_eq!(
                    value,
                    resolve_damage_amount_from_params(ctx, sa),
                    "compiled DealDamage amount diverged from string params"
                );
                return value;
            }
        }
    }

    resolve_damage_amount_from_params(ctx, sa)
}

fn resolve_amount_expr(ctx: &EffectContext, sa: &SpellAbility, amount: &AmountExpr) -> Option<i32> {
    match amount {
        AmountExpr::Literal(value) => Some(*value),
        AmountExpr::X => Some(resolve_x_amount(ctx, sa)),
        AmountExpr::SVar(name) => Some(resolve_svar_amount(ctx, sa, name)),
        AmountExpr::Raw(_) => None,
    }
}

fn resolve_damage_amount_from_params(ctx: &EffectContext, sa: &SpellAbility) -> i32 {
    resolve_numeric_svar(ctx.game, sa, keys::NUM_DMG, 0)
}

fn resolve_x_amount(ctx: &EffectContext, sa: &SpellAbility) -> i32 {
    if let Some(svar_expr) = crate::ability::ability_utils::get_s_var(sa, ctx.game, "X") {
        return evaluate_svar_expr(ctx, sa, svar_expr);
    }
    sa.x_mana_cost_paid as i32
}

fn resolve_svar_amount(ctx: &EffectContext, sa: &SpellAbility, var_name: &str) -> i32 {
    if let Some(expr) = crate::ability::ability_utils::get_s_var(sa, ctx.game, var_name) {
        return evaluate_svar_expr(ctx, sa, expr);
    }

    0
}

/// Evaluate a simple SVar expression string.
/// Mirrors Java's `AbilityUtils.calculateAmount` for common SVar patterns.
fn evaluate_svar_expr(ctx: &EffectContext, sa: &SpellAbility, expr: &str) -> i32 {
    // Count$ expressions — delegate to shared game-aware resolver
    if expr.starts_with("Count$") {
        if let Some(source_id) = sa.source {
            return crate::svar::resolve_count_svar_for_sa(
                expr,
                ctx.game,
                source_id,
                sa.activating_player,
                sa,
            );
        }
    }
    if expr.starts_with("SVar$") {
        if let Some(source_id) = sa.source {
            return crate::svar::resolve_svar_expression(
                expr,
                ctx.game,
                source_id,
                sa.activating_player,
                sa,
            );
        }
    }
    // Any `<PaidKey>$<property>` the cost payment actually recorded goes to the shared
    // resolver, which is where Java's `handlePaid` lives; this local evaluator only
    // knows the two Sacrificed forms below.
    if let Some((key, _)) = expr.split_once('$') {
        if sa.paid_hash.contains_key(key) {
            if let Some(source_id) = sa.source {
                return crate::svar::resolve_svar_expression(
                    expr,
                    ctx.game,
                    source_id,
                    sa.activating_player,
                    sa,
                );
            }
        }
    }
    // Sacrificed$CardPower / Sacrificed$CardToughness — LKI from cost payment.
    // Used by Rite of Consumption: SVar:X:Sacrificed$CardPower
    if expr == "Sacrificed$CardPower" || expr == "Sacrificed$CardToughness" {
        if let Some(sac_id) = ctx.game.last_sacrificed_card {
            let sac_card = ctx.game.card(sac_id);
            let val = if expr.ends_with("Power") {
                sac_card
                    .lki_power
                    .unwrap_or(sac_card.base_power.unwrap_or(0))
            } else {
                sac_card
                    .lki_toughness
                    .unwrap_or(sac_card.base_toughness.unwrap_or(0))
            };
            return val;
        }
        return 0;
    }
    if expr == "TriggeredCard$CardPower" || expr == "TriggeredCard$CardToughness" {
        let trigger_value_key = if expr.ends_with("CardPower") {
            "TriggeredCardPower"
        } else {
            "TriggeredCardToughness"
        };
        if let Some(value) = crate::ability::ability_key::from_string(trigger_value_key)
            .and_then(|key| sa.get_triggering_value(key))
            .and_then(|value| value.to_trigger_text().trim().parse::<i32>().ok())
        {
            return value;
        }

        let triggered_card = sa
            .get_triggering_card(crate::ability::AbilityKey::Card)
            .or(sa.trigger_source);
        if let Some(card_id) = triggered_card {
            return if expr.ends_with("CardPower") {
                crate::lki::resolve_lki_power(ctx.game, card_id)
            } else {
                crate::lki::resolve_lki_toughness(ctx.game, card_id)
            };
        }
        return 0;
    }
    match expr {
        // X mana cost paid value
        "Count$xPaid" | "Count$XPaid" => sa.x_mana_cost_paid as i32,
        // Power / toughness of the parent SA's chosen target card.
        // Used by Ram Through: SVar:X:ParentTargeted$CardPower
        "ParentTargeted$CardPower" => ctx
            .parent_target_card
            .map(|id| ctx.game.card(id).power())
            .unwrap_or(0),
        "ParentTargeted$CardToughness" => ctx
            .parent_target_card
            .map(|id| ctx.game.card(id).toughness())
            .unwrap_or(0),
        _ => crate::svar::resolve_numeric_value(ctx.game, sa, expr, 0),
    }
}
