use forge_foundation::ZoneType;

use super::{matches_valid_cards_for_sa, parse_counter_type, resolve_numeric_svar, EffectContext};
use crate::agent::GameEntity;
use crate::card::CounterType;
use crate::game_entity_counter_table::GameEntityCounterTable;
use crate::ids::CardId;

/// `SP$ PutCounterAll` — put counters on all matching permanents.
///
/// Mirrors Java's `CountersPutAllEffect.java`.
/// - `CounterType$` — type of counter (default P1P1).
/// - `CounterNum$` — number of counters to add (default 1).
/// - `ValidCards$` — filter for which cards receive counters.
/// - `ValidZone$` — zone to search (default Battlefield).
///
/// # Card script examples
/// ```text
/// A:SP$ PutCounterAll | CounterType$ P1P1 | CounterNum$ 1 | ValidCards$ Creature.YouCtrl
/// A:SP$ PutCounterAll | CounterType$ CHARGE | CounterNum$ 2 | ValidCards$ Artifact
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `CountersPutAllEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(CountersPutAllEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let counter_type = sa.ir.counter_type.clone().unwrap_or(CounterType::P1P1);
    let count = resolve_numeric_svar(ctx.game, sa, "CounterNum", 1);
    if count == 0 {
        return;
    }

    let valid_cards = sa.ir.valid_cards_selector.as_ref();
    let zone = sa.ir.valid_zone.unwrap_or(ZoneType::Battlefield);

    let player_ids = match sa
        .target_chosen
        .target_player
        .filter(|_| sa.uses_targeting())
    {
        Some(pid) => vec![pid],
        None => ctx.game.player_order.clone(),
    };
    let mut targets = valid_cards_in_zone(ctx, sa, &player_ids, zone, valid_cards);

    let placer = sa.activating_player;
    let mut table = GameEntityCounterTable::default();
    put_counters(
        ctx,
        &targets,
        zone,
        &counter_type,
        count,
        placer,
        &mut table,
    );

    let valid_cards2 = crate::parsing::raw_get(&sa.ability_text, "ValidCards2");
    let counter_type2 = crate::parsing::raw_get(&sa.ability_text, "CounterType2");
    if valid_cards2.is_some()
        || counter_type2.is_some()
        || crate::parsing::raw_has_key(&sa.ability_text, "CounterNum2")
    {
        let counter_type2 = counter_type2.map_or_else(|| counter_type.clone(), parse_counter_type);
        let zone2 = crate::parsing::raw_get(&sa.ability_text, "ValidZone2")
            .and_then(crate::zone::zone_type::smart_value_of)
            .unwrap_or(zone);
        if let Some(valid2) = valid_cards2 {
            let selector2 = crate::parsing::cached_compiled_selector(valid2);
            targets = valid_cards_in_zone(ctx, sa, &player_ids, zone2, Some(&selector2));
        }
        let count2 = resolve_numeric_svar(ctx.game, sa, "CounterNum2", count);
        put_counters(
            ctx,
            &targets,
            zone2,
            &counter_type2,
            count2,
            placer,
            &mut table,
        );
    }

    table.replace_counter_effect(
        ctx.game,
        Some(ctx.trigger_handler),
        Some(ctx.agents),
        Some(sa),
        true,
        Default::default(),
    );
}

fn valid_cards_in_zone(
    ctx: &EffectContext,
    sa: &crate::spellability::SpellAbility,
    player_ids: &[crate::ids::PlayerId],
    zone: ZoneType,
    valid_cards: Option<&crate::parsing::CompiledSelector>,
) -> Vec<CardId> {
    let mut cards = Vec::new();
    for &pid in player_ids {
        for &cid in ctx.game.cards_in_zone(zone, pid) {
            if matches_valid_cards_for_sa(ctx.game, sa, ctx.game.card(cid), valid_cards, "Creature")
            {
                cards.push(cid);
            }
        }
    }
    cards
}

fn put_counters(
    ctx: &EffectContext,
    cards: &[CardId],
    zone: ZoneType,
    counter_type: &CounterType,
    count: i32,
    placer: crate::ids::PlayerId,
    table: &mut GameEntityCounterTable,
) {
    for &card_id in cards {
        if ctx.game.card(card_id).zone == zone {
            if crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
                &ctx.game.cards,
                ctx.game.card(card_id),
                counter_type,
            ) {
                continue;
            }
            table.put(
                Some(placer),
                GameEntity::Card(card_id),
                counter_type.clone(),
                count,
            );
        }
    }
}
