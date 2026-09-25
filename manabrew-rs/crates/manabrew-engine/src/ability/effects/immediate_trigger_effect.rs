//! ImmediateTrigger effect — fire a trigger immediately without waiting.
//!
//! Mirrors Java's `ImmediateTriggerEffect.resolve()` which registers
//! a delayed trigger that fires as soon as possible through normal
//! trigger processing.

use super::EffectContext;
use crate::agent::GameEntity;
use crate::trigger::DelayedTrigger;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ImmediateTriggerEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ImmediateTriggerEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let amount = sa
        .ir
        .trigger_amount
        .as_deref()
        .map_or(1, |raw| super::resolve_numeric_value(ctx.game, sa, raw, 0));
    if amount <= 0 {
        return;
    }

    let mut remembered = Vec::new();
    if let Some(remember_def) = sa.ir.remember_objects.as_deref() {
        for defined in remember_def.split(" & ") {
            let (players, mut cards) =
                crate::ability::ability_utils::get_defined_entities(defined, sa, ctx.game);
            if cards.is_empty() && defined == "Targeted" {
                cards.extend(ctx.parent_target_card);
            }
            remembered.extend(players.into_iter().map(GameEntity::Player));
            remembered.extend(cards.into_iter().map(GameEntity::Card));
        }
    }

    let Some(execute_name) = sa.ir.execute.as_deref() else {
        return;
    };
    let Some(source_id) = sa.source else {
        return;
    };
    if crate::ability::ability_utils::get_s_var(sa, ctx.game, execute_name).is_none() {
        return;
    }
    let remembered_amount = sa
        .ir
        .remember_svar_amount
        .as_deref()
        .and_then(|svar| crate::ability::ability_utils::get_s_var(sa, ctx.game, svar))
        .map_or(0, |expr| {
            crate::svar::resolve_svar_expression(
                expr,
                ctx.game,
                source_id,
                sa.activating_player,
                sa,
            )
        });
    for i in 0..amount as usize {
        let entities = if sa.ir.remember_each {
            remembered.get(i..=i).unwrap_or_default()
        } else {
            &remembered[..]
        };
        let mut remembered_cards = Vec::new();
        let mut remembered_players = Vec::new();
        for entity in entities {
            match *entity {
                GameEntity::Player(player) => remembered_players.push(player),
                GameEntity::Card(card) => remembered_cards.push(card),
            }
        }
        let delayed = DelayedTrigger {
            mode: TriggerType::Immediate,
            trigger_mode: Box::new(crate::trigger::trigger_immediate::TriggerImmediate),
            params: crate::parsing::Params::default(),
            execute_svar: execute_name.to_string(),
            controller: sa.activating_player,
            source_card: source_id,
            created_turn: ctx.game.turn.turn_number,
            created_phase: ctx.game.turn.phase,
            target_card: None,
            remembered_amount,
            remembered_cards,
            remembered_players,
            remembered_lki_cards: Vec::new(),
            target_card_zone_timestamp: None,
            sort_after_active: false,
            trigger_order: None,
            source_timestamp: None,
            spawning_ability: Some(sa.clone()),
        };
        ctx.trigger_handler.register_delayed_trigger(delayed);
    }
}
