use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;
use crate::agent::BinaryChoiceKind;
use crate::agent::DecisionContext;
use crate::ids::CardId;

/// `SP$ TapOrUntap` — choose tap or untap for the targeted/defined permanent.
///
/// Mirrors Java `TapOrUntapEffect.java` binary prompt behavior.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `TapOrUntapEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(TapOrUntapEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let controller = sa.activating_player;
    let _source_name = sa
        .source
        .map(|cid| ctx.game.card(cid).card_name.clone())
        .unwrap_or_else(|| "Ability".to_string());

    let mut candidates: Vec<CardId> = Vec::new();
    if let Some(target) = sa.target_chosen.target_card {
        candidates.push(target);
    } else if let Some(source) = sa.source {
        if matches!(sa.defined_ref(), Some(DefinedRef::SelfCard)) {
            candidates.push(source);
        }
    }

    let mut untapped = Vec::new();
    for card_id in candidates {
        if ctx.game.card(card_id).zone != ZoneType::Battlefield {
            continue;
        }
        let should_tap = if sa.ir.toggle {
            !ctx.game.card(card_id).tapped
        } else {
            let prompt = format!("Tap or untap {}?", ctx.game.card(card_id).card_name);
            ctx.agents[controller.index()].choose_binary(
                DecisionContext::new(ctx.game, ctx.mana_pools),
                controller,
                &prompt,
                BinaryChoiceKind::TapOrUntap,
                None,
                sa.source,
                sa.api,
            )
        };

        if should_tap {
            if ctx.game.tap(card_id) {
                ctx.trigger_handler.run_trigger(
                    crate::trigger::TriggerType::Taps,
                    crate::event::RunParams {
                        card: Some(card_id),
                        player: Some(controller),
                        ..Default::default()
                    },
                    false,
                );
            }
        } else if ctx.game.untap(card_id, ctx.trigger_handler) {
            untapped.push(card_id);
        }
    }
    if !untapped.is_empty() {
        ctx.trigger_handler.run_trigger(
            crate::trigger::TriggerType::UntapAll,
            crate::event::RunParams {
                map: Some(vec![(controller, untapped)]),
                ..Default::default()
            },
            false,
        );
    }
}
