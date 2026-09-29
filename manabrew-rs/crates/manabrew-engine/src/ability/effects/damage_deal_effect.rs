use forge_foundation::ZoneType;

use super::{resolve_numeric_svar, EffectContext};
use crate::ability::ability_ir::EffectIr;
use crate::agent::DecisionContext;
use crate::card::card_damage_map::{CardDamageMap, DamageTarget};
use crate::card::card_util;
use crate::game_entity_counter_table::GameEntityCounterTable;
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
            DecisionContext::new(ctx.game, ctx.mana_pools),
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

    let mut damage_map = CardDamageMap::default();
    for source in sources.into_iter().filter(|_| damage > 0) {
        for target in damage_targets(ctx.game, sa, &target_players, &target_cards) {
            damage_map.put(source, target, damage);
        }
    }
    if use_damage_map {
        if let Some(pending) = ctx.game.pending_damage_map.as_mut() {
            for (source, target, amount) in damage_map.entries() {
                pending.put(source, target, amount);
            }
        }
    } else {
        ctx.deal_damage(
            &mut damage_map,
            &mut CardDamageMap::default(),
            &mut GameEntityCounterTable::default(),
            Some(sa),
        );
        ctx.trigger_handler.flush_waiting_triggers(ctx.game);
    }

    let _ = crate::ability::spell_ability_effect::replace_dying(ctx.game, sa);
}

pub(super) fn excess_damage_value(
    game: &crate::game::GameState,
    card_id: crate::ids::CardId,
    source: crate::ids::CardId,
) -> i32 {
    let card = game.card(card_id);
    if card.is_creature() && game.get_change_zone_lki_info(source).has_deathtouch() {
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

pub(super) fn excess_svar_condition(
    game: &crate::game::GameState,
    sa: &SpellAbility,
    card_id: crate::ids::CardId,
) -> bool {
    match crate::parsing::raw_get(&sa.ability_text, "ExcessSVarCondition") {
        Some(valid) => super::matches_valid_cards_for_sa(game, sa, game.card(card_id), None, valid),
        None => true,
    }
}

fn damage_targets(
    game: &crate::game::GameState,
    sa: &SpellAbility,
    target_players: &[crate::ids::PlayerId],
    target_cards: &[crate::ids::CardId],
) -> Vec<DamageTarget> {
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
        return game
            .player_order
            .iter()
            .flat_map(|&pid| game.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .filter(|&cid| {
                super::matches_valid_cards_for_sa(
                    game,
                    sa,
                    game.card(cid),
                    valid_tgts_selector,
                    valid_tgts,
                )
            })
            .map(DamageTarget::Card)
            .collect();
    }
    target_players
        .iter()
        .copied()
        .map(DamageTarget::Player)
        .chain(
            target_cards
                .iter()
                .copied()
                .filter(|&cid| {
                    let card = game.card(cid);
                    card.zone == ZoneType::Battlefield && !card.phased_out
                })
                .map(DamageTarget::Card),
        )
        .collect()
}

/// CR 702.15e: one gain for the whole event, as the lifelink step of `GameAction.dealDamage`.
/// Mirrors the combat lifelink path in `combat/mod.rs` — the can't-gain check and the
/// GainLife replacement chain apply here too.
pub(crate) fn gain_life_from_lifelink(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    source: crate::ids::CardId,
    lifelink_dealt: i32,
) {
    let game = &mut *ctx.game;
    let source_lki = game.get_change_zone_lki_info(source);
    if lifelink_dealt <= 0 || !source_lki.has_lifelink() {
        return;
    }
    let controller = source_lki.controller;
    if crate::staticability::static_ability_cant_gain_lose_pay_life::cant_gain_life(
        game, controller,
    ) {
        return;
    }
    let mut gl_event = crate::replacement::replacement_handler::ReplacementEvent::GainLife {
        player: controller,
        amount: lifelink_dealt,
    };
    let gl_result =
        crate::replacement::replacement_handler::apply_replacements(game, &mut gl_event);
    if gl_result == crate::replacement::ReplacementResult::Skipped
        || gl_result == crate::replacement::ReplacementResult::Replaced
    {
        return;
    }
    let final_amount =
        if let crate::replacement::replacement_handler::ReplacementEvent::GainLife {
            amount, ..
        } = gl_event
        {
            amount
        } else {
            lifelink_dealt
        };
    if final_amount > 0 {
        game.player_gain_life(controller, final_amount);
        game.player_add_team_life_gained(controller, final_amount);
        ctx.trigger_handler.run_trigger(
            crate::trigger::TriggerType::LifeGained,
            crate::event::RunParams {
                player: Some(controller),
                life_amount: Some(final_amount),
                first_time: Some(game.player(controller).life_gained_this_turn == final_amount),
                source_card: Some(source),
                source_sa: Some(sa.clone()),
                ..Default::default()
            },
            false,
        );
    }
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
        return crate::lki::resolve_triggered_card_lki_svar(ctx.game, sa, expr).unwrap_or(0);
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
