use forge_foundation::ZoneType;

use super::EffectContext;
use crate::card::card_damage_map::DamageTarget;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::trigger::TriggerType;

/// SP$/DB$ Fight — two creatures deal damage to each other equal to their power.
///
/// Mirrors Java's `FightEffect.resolve()`.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `FightEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(FightEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let use_damage_map = ctx.game.pending_damage_map.is_some() || sa.ir.damage_map;
    if sa.ir.damage_map {
        ctx.game.ensure_pending_damage_maps();
    }

    let mut sa_with_parent;
    let sa = if sa.parent_targeting_card.is_none() && ctx.parent_target_card.is_some() {
        sa_with_parent = sa.clone();
        sa_with_parent.parent_targeting_card = ctx.parent_target_card;
        &sa_with_parent
    } else {
        sa
    };
    let fighters = get_fighters(ctx.game, sa);
    let [source, target] = fighters[..] else {
        return;
    };

    if sa.ir.optional {
        let decider = sa
            .source
            .map(|cid| ctx.game.card(cid).controller)
            .unwrap_or(sa.activating_player);
        ctx.agents[decider.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[decider.index()].confirm_action(
            decider,
            Some("Fight"),
            "Would you like those creatures to fight?",
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }

    let source_power = ctx.game.card(source).power();
    let target_power = ctx.game.card(target).power();

    // Track damage sources for DamagedBy trigger filters
    if !ctx
        .game
        .card(target)
        .damage_sources_this_turn
        .contains(&source)
    {
        ctx.game
            .card_mut(target)
            .damage_sources_this_turn
            .push(source);
    }
    if !ctx
        .game
        .card(source)
        .damage_sources_this_turn
        .contains(&target)
    {
        ctx.game
            .card_mut(source)
            .damage_sources_this_turn
            .push(target);
    }
    // Deal damage simultaneously
    if use_damage_map {
        if let Some(map) = ctx.game.pending_damage_map.as_mut() {
            map.put(source, DamageTarget::Card(target), source_power);
            map.put(target, DamageTarget::Card(source), target_power);
        }
    } else {
        ctx.game.deal_damage_to_card(target, source_power);
        ctx.game.deal_damage_to_card(source, target_power);
    }

    // Fire per-fighter and batched fight triggers (matches Java FightEffect).
    ctx.trigger_handler.run_trigger(
        TriggerType::Fight,
        RunParams {
            card: Some(source),
            card2: Some(target),
            ..Default::default()
        },
        false,
    );
    ctx.trigger_handler.run_trigger(
        TriggerType::Fight,
        RunParams {
            card: Some(target),
            card2: Some(source),
            ..Default::default()
        },
        false,
    );
    ctx.trigger_handler.run_trigger(
        TriggerType::FightOnce,
        RunParams {
            card: Some(source),
            card2: Some(target),
            ..Default::default()
        },
        false,
    );

    let _ = crate::ability::spell_ability_effect::replace_dying(ctx.game, sa);
}

fn get_fighters(
    game: &crate::game::GameState,
    sa: &crate::spellability::SpellAbility,
) -> Vec<CardId> {
    let tgts: Vec<CardId> = if sa.uses_targeting() {
        sa.target_chosen.target_card.into_iter().collect()
    } else {
        Vec::new()
    };
    let mut fighter1 = tgts.first().copied();
    let mut fighter2 = None;
    if sa.ir.defined.is_some() {
        let defined: Vec<CardId> =
            crate::ability::spell_ability_effect::get_defined_cards_or_targeted(game, sa)
                .into_iter()
                .filter(|&cid| {
                    let card = game.card(cid);
                    card.zone == ZoneType::Battlefield && !card.phased_out && card.is_creature()
                })
                .collect();
        if !defined.is_empty() {
            if defined.len() > 1 && fighter1.is_none() {
                fighter1 = Some(defined[0]);
                fighter2 = Some(defined[1]);
            } else {
                fighter2 = fighter1;
                fighter1 = Some(defined[0]);
            }
        }
    } else if tgts.len() > 1 {
        fighter2 = Some(tgts[1]);
    }
    fighter1.into_iter().chain(fighter2).collect()
}
