//! Discover effect — exile from library until a nonland card with CMC ≤ N.
//!
//! Ported 1:1 from Java's `DiscoverEffect.java`.
//! Discover N: Exile cards from the top of your library until you exile a nonland
//! card with mana value N or less. Cast it without paying its mana cost or put it
//! into your hand. Put the rest on the bottom of your library in a random order.

use forge_foundation::ZoneType;

use super::{emit_zone_trigger, EffectContext};
use crate::agent::DecisionContext;
use crate::event::RunParams;
use crate::ids::{CardId, PlayerId};
use crate::parsing::keys;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DiscoverEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DiscoverEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let num = discover_num(ctx, sa);
    for player in
        crate::ability::spell_ability_effect::get_defined_players_or_targeted(ctx.game, sa)
    {
        let (found, rest) = exile_until_found(ctx, sa, player, num);
        if let Some(card_id) = found {
            confirm_cast(ctx, card_id, player);
            put_into_hand(ctx, card_id, player);
        }
        put_rest_on_bottom(ctx, rest, player);
        run_discover_trigger(ctx, player, num);
    }
}

pub(crate) fn discover_num(ctx: &EffectContext, sa: &SpellAbility) -> i32 {
    super::resolve_numeric_svar(ctx.game, sa, "Num", 1).max(0)
}

pub(crate) fn exile_until_found(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    player: PlayerId,
    max_cmc: i32,
) -> (Option<CardId>, Vec<CardId>) {
    let mut rest: Vec<CardId> = Vec::new();
    loop {
        let lib = ctx.game.cards_in_zone(ZoneType::Library, player).to_vec();
        let Some(&top) = lib.last() else {
            return (None, rest);
        };

        let card = ctx.game.card(top);
        let is_land = card
            .type_line
            .core_types
            .iter()
            .any(|ct| matches!(ct, forge_foundation::CoreType::Land));
        let cmc = card.mana_cost.cmc();

        let old_zone = ctx.game.card(top).zone;
        ctx.exile(top, Some(sa));
        emit_zone_trigger(ctx.trigger_handler, top, old_zone, ZoneType::Exile);

        if !is_land && cmc <= max_cmc {
            if sa.param_is_true(keys::REMEMBER_DISCOVERED) {
                if let Some(sid) = sa.source {
                    ctx.game.card_mut(sid).add_remembered_card(top);
                }
            }
            return (Some(top), rest);
        }
        rest.push(top);
    }
}

pub(crate) fn confirm_cast(ctx: &mut EffectContext, card_id: CardId, player: PlayerId) -> bool {
    let card_name = ctx.game.card(card_id).card_name.clone();
    let cast_label = "Cast without paying its mana cost";
    let hand_label = "Put into your hand";
    ctx.agents[player.index()].snapshot_state(ctx.game, ctx.mana_pools);
    ctx.agents[player.index()].confirm_action(
        DecisionContext::new(ctx.game, ctx.mana_pools),
        player,
        Some("CastFromEffect"),
        &format!("{card_name}: {cast_label} or {hand_label}?"),
        &[cast_label.to_string(), hand_label.to_string()],
        Some(card_id),
        None,
    )
}

pub(crate) fn put_into_hand(ctx: &mut EffectContext, card_id: CardId, player: PlayerId) {
    let old = ctx.game.card(card_id).zone;
    ctx.move_card(card_id, ZoneType::Hand, player);
    emit_zone_trigger(ctx.trigger_handler, card_id, old, ZoneType::Hand);
}

pub(crate) fn put_rest_on_bottom(ctx: &mut EffectContext, mut rest: Vec<CardId>, player: PlayerId) {
    ctx.rng.shuffle_cards(&mut rest);
    for card_id in rest {
        let old = ctx.game.card(card_id).zone;
        ctx.move_card(card_id, ZoneType::Library, player);
        ctx.game
            .reorder_card_in_zone(ZoneType::Library, player, card_id, 0);
        emit_zone_trigger(ctx.trigger_handler, card_id, old, ZoneType::Library);
    }
}

pub(crate) fn run_discover_trigger(ctx: &mut EffectContext, player: PlayerId, num: i32) {
    ctx.trigger_handler.run_trigger(
        TriggerType::Discover,
        RunParams {
            player: Some(player),
            num: Some(num),
            ..Default::default()
        },
        false,
    );
}
