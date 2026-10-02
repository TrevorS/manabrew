use forge_foundation::ZoneType;

use super::{emit_zone_trigger, EffectContext};
use crate::agent::DecisionContext;
use crate::card::CounterType;
use crate::event::RunParams;
use crate::parsing::keys;
use crate::replacement::replacement_handler::{
    apply_replacements_with_agents_and_runtime, ReplacementEvent, ReplacementRuntime,
};
use crate::replacement::ReplacementResult;
use crate::trigger::TriggerType;

/// `SP$ Explore` — target creature explores.
///
/// Mirrors Java's `ExploreEffect.java`.
/// Explore: Reveal the top card of your library. If it's a land, put it into your hand.
/// Otherwise, put a +1/+1 counter on this creature, then you may put the card into
/// your graveyard.
///
/// # Card script examples
/// ```text
/// A:SP$ Explore | Defined$ Self
/// A:SP$ Explore | Defined$ Targeted | Num$ 2
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ExploreEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ExploreEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let controller = sa.activating_player;

    let mut explorers = crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa);
    if explorers.is_empty() {
        explorers.extend(ctx.parent_target_card);
    }
    if explorers.is_empty()
        && !sa.uses_targeting()
        && !crate::parsing::raw_has_key(&sa.ability_text, keys::DEFINED)
    {
        explorers.extend(sa.source);
    }
    for explorer_id in explorers {
        explore_one(ctx, sa, controller, explorer_id);
    }
}

fn explore_one(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    controller: crate::ids::PlayerId,
    explorer_id: crate::ids::CardId,
) {
    // Java `ExploreEffect.resolve` reads the count first and runs the Explore
    // replacement once per explore, so `Num$ 0` explores no times at all.
    let amount = super::resolve_numeric_svar(ctx.game, sa, keys::NUM, 1);
    let explorer_timestamp = ctx.game.card(explorer_id).zone_timestamp;

    for _ in 0..amount {
        let mut event = ReplacementEvent::Explore { card: explorer_id };
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
        if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
            continue;
        }
        ctx.game.player_record_explore(controller, 1);

        let top_card = ctx
            .game
            .cards_in_zone(ZoneType::Library, controller)
            .last()
            .copied();
        let mut revealed_land = false;
        if let Some(top_card) = top_card {
            if ctx.game.card(top_card).is_land() {
                let owner = ctx.game.card(top_card).owner;
                ctx.move_card(top_card, ZoneType::Hand, owner);
                emit_zone_trigger(
                    ctx.trigger_handler,
                    top_card,
                    ZoneType::Library,
                    ZoneType::Hand,
                );
                revealed_land = true;
            } else {
                // Java's ExploreEffect calls controller.confirmAction() which in the
                // harness DeterministicController uses a random boolean (pickBool).
                let card_name = ctx.game.card(top_card).card_name.clone();
                let msg = format!("Put {card_name} into your graveyard?");
                let put_in_gy = ctx.agents[controller.index()].confirm_action(
                    DecisionContext::new(ctx.game, ctx.mana_pools),
                    controller,
                    None,
                    &msg,
                    &[],
                    sa.source,
                    Some(crate::ability::api_type::ApiType::Explore),
                );
                if put_in_gy {
                    let owner = ctx.game.card(top_card).owner;
                    ctx.move_card(top_card, ZoneType::Graveyard, owner);
                    emit_zone_trigger(
                        ctx.trigger_handler,
                        top_card,
                        ZoneType::Library,
                        ZoneType::Graveyard,
                    );
                }
            }
        }
        let explorer = ctx.game.card(explorer_id);
        if !revealed_land
            && explorer.zone == ZoneType::Battlefield
            && explorer.zone_timestamp == explorer_timestamp
            && !crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
                ctx.game,
                explorer,
                &CounterType::P1P1,
            )
        {
            ctx.add_counter(explorer_id, &CounterType::P1P1, 1, sa, RunParams::default());
        }
        ctx.trigger_handler.run_trigger(
            TriggerType::Explored,
            RunParams {
                card: Some(explorer_id),
                explored: top_card,
                ..Default::default()
            },
            false,
        );
    }
}
