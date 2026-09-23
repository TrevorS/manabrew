use forge_foundation::ZoneType;

use super::{emit_zone_trigger, resolve_numeric_svar, EffectContext};
use crate::agent::GameEntity;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::replacement::replacement_handler::{
    apply_replacements_with_agents_and_runtime, ReplacementEvent, ReplacementRuntime,
};
use crate::replacement::ReplacementResult;
use crate::trigger::TriggerType;

/// `DB$ Connive` — target creature connives N times.
///
/// Connive: draw N cards, then discard N cards. For each nonland card
/// discarded this way, put a +1/+1 counter on the conniving creature.
///
/// Mirrors Java's `ConniveEffect.resolve()`.
///
/// # Params
/// - `ConniveNum` — number of times to connive (default: 1)
/// - Target or `Defined$` — the creature that connives
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ConniveEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ConniveEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let num = resolve_numeric_svar(ctx.game, sa, keys::CONNIVE_NUM, 1).max(0) as usize;

    let to_connive =
        if sa.uses_targeting() || crate::parsing::raw_has_key(&sa.ability_text, keys::DEFINED) {
            crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa)
        } else {
            sa.source.into_iter().collect()
        };
    if to_connive.is_empty() {
        return;
    }

    let player_order = ctx.game.player_order.clone();
    let start = player_order
        .iter()
        .position(|p| *p == ctx.game.turn.active_player)
        .unwrap_or(0);
    for idx in 0..player_order.len() {
        let p = player_order[(start + idx) % player_order.len()];
        let mut connivers: Vec<CardId> = to_connive
            .iter()
            .copied()
            .filter(|c| ctx.game.card(*c).controller == p)
            .collect();
        while !connivers.is_empty() {
            let conniver = if connivers.len() > 1 {
                let options: Vec<GameEntity> =
                    connivers.iter().copied().map(GameEntity::Card).collect();
                match ctx.agents[p.index()].choose_single_entity_for_effect(p, &options, false) {
                    Some(GameEntity::Card(card)) => card,
                    _ => connivers[0],
                }
            } else {
                connivers[0]
            };
            connivers.retain(|c| *c != conniver);

            let mut event = ReplacementEvent::Connive { card: conniver };
            let mut runtime = ReplacementRuntime {
                trigger_handler: ctx.trigger_handler,
                token_templates: ctx.token_templates,
                token_art_variants: ctx.token_art_variants,
                token_fallback: ctx.token_fallback,
                edition_dates: ctx.edition_dates,
                mana_pools: ctx.mana_pools,
                rng: ctx.rng,
            };
            let result = apply_replacements_with_agents_and_runtime(
                ctx.game,
                ctx.agents,
                &mut runtime,
                &mut event,
            );
            if result != ReplacementResult::NotReplaced {
                continue;
            }
            connive_one(ctx, sa, num, conniver);
        }
    }
}

fn connive_one(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    num: usize,
    conniver_id: CardId,
) {
    // Java's `ConniveEffect.resolve` draws and discards unconditionally once a conniver is
    // resolved; only the +1/+1 counter placement checks whether it is still on the
    // battlefield, further down. A conniver that has already left (the legend rule
    // sacrificing the trigger's own host is the common case) still draws and discards,
    // just gains no counter.
    let controller = ctx.game.card(conniver_id).controller;

    // Draw N cards.
    for _ in 0..num {
        super::draw_effect::draw_card(ctx, controller);
    }

    // Discard N cards from hand.
    let hand: Vec<_> = ctx.game.cards_in_zone(ZoneType::Hand, controller).to_vec();

    if hand.is_empty() {
        return;
    }

    let amt = hand.len().min(num);
    let to_discard = ctx.agents[controller.index()].choose_discard(controller, &hand, amt);

    // Count nonland cards discarded (for +1/+1 counters).
    let mut nonland_count = 0i32;
    for card_id in &to_discard {
        if ctx.game.card(*card_id).zone == ZoneType::Hand {
            if !ctx.game.card(*card_id).is_land() {
                nonland_count += 1;
            }
            ctx.game.player_record_discard(controller, 1);
            let owner = ctx.game.card(*card_id).owner;
            ctx.move_card(*card_id, ZoneType::Graveyard, owner);
            ctx.game.card_mut(*card_id).set_discarded(true);
            emit_zone_trigger(
                ctx.trigger_handler,
                *card_id,
                ZoneType::Hand,
                ZoneType::Graveyard,
            );
            ctx.trigger_handler.run_trigger(
                TriggerType::Discarded,
                RunParams {
                    card: Some(*card_id),
                    player: Some(controller),
                    ..Default::default()
                },
                false,
            );
        }
    }

    // Put +1/+1 counters on the conniver for each nonland card discarded,
    // but only if it's still on the battlefield.
    if nonland_count > 0 && ctx.game.card(conniver_id).zone == ZoneType::Battlefield {
        ctx.add_counter(
            conniver_id,
            &crate::card::CounterType::P1P1,
            nonland_count,
            sa,
            RunParams {
                player: Some(controller),
                ..Default::default()
            },
        );
    }
}
