use super::EffectContext;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::replacement::replacement_handler::{apply_replacements, ReplacementEvent};
use crate::replacement::ReplacementResult;
use crate::spellability::SpellAbilityMode;

/// Mirrors Java's `SetStateEffect.java`.
///
/// `DB$ SetState | Defined$ Self | Mode$ Transform`
///
/// Optionally gated by:
///   `ConditionDefined$ Remembered | ConditionPresent$ Card.Instant,Card.Sorcery | ConditionCompare$ EQ1`
///
/// If the condition passes, transforms the source DFC card to its other face.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `SetStateEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(SetStateEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let mode = sa.ir.mode.as_ref();
    for card_id in crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa) {
        let face_mode = matches!(
            mode,
            Some(SpellAbilityMode::TurnFaceUp) | Some(SpellAbilityMode::TurnFaceDown)
        );
        if !face_mode
            && ctx.game.card(card_id).zone != forge_foundation::ZoneType::Battlefield
            && !crate::parsing::raw_has_key(&sa.ability_text, "ETB")
        {
            continue;
        }
        if crate::parsing::raw_has_key(&sa.ability_text, "RevealFirst") {
            let mut lki = ctx.game.card(card_id).clone();
            lki.turn_face_up();
            if let (Some(valid_new_face), Some(source_id)) = (
                crate::parsing::raw_get(&sa.ability_text, "ValidNewFace"),
                sa.source,
            ) {
                let source = ctx.game.card(source_id);
                if !valid_new_face.split(',').any(|valid| {
                    crate::card::valid_filter::matches_valid_card_selector_in_game(
                        &crate::parsing::cached_compiled_selector(valid.trim()),
                        &lki,
                        source,
                        ctx.game,
                    )
                }) {
                    continue;
                }
            }
        }
        if sa.param_is_true(keys::OPTIONAL) {
            let message = format!("Transform {}?", ctx.game.card(card_id).card_name);
            if !ctx.agents[sa.activating_player.index()].confirm_action(
                sa.activating_player,
                Some("Random"),
                &message,
                &[],
                Some(card_id),
                sa.api,
            ) {
                return;
            }
        }
        set_state_for_card(ctx, sa, card_id, mode);
    }
}

fn set_state_for_card(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    card_id: CardId,
    mode: Option<&SpellAbilityMode>,
) {
    match mode {
        Some(SpellAbilityMode::Transform) => {
            // Evaluate optional condition.
            if let Some(cond_defined) = sa.ir.condition_defined.as_ref() {
                if cond_defined.refs.first().is_some_and(|defined| {
                    matches!(defined, crate::ability::ability_ir::DefinedRef::Remembered)
                }) {
                    let cond_present = sa.ir.condition_present.clone().unwrap_or_default();
                    let cond_compare = sa.ir.condition_compare.clone().unwrap_or_default();

                    let remembered: Vec<CardId> = ctx.game.card(card_id).remembered_cards.clone();
                    let match_count = remembered
                        .iter()
                        .filter(|&&cid| matches_type_filter(ctx, cid, &cond_present))
                        .count();

                    if !evaluate_compare(&cond_compare, match_count) {
                        return; // Condition not met.
                    }
                }
            }

            // Run Transform replacement effects before transforming.
            let mut transform_event = ReplacementEvent::Transform { card: card_id };
            let transform_result = apply_replacements(ctx.game, &mut transform_event);
            if transform_result == ReplacementResult::Skipped
                || transform_result == ReplacementResult::Replaced
            {
                return;
            }

            // Perform the transform.
            ctx.game.card_mut(card_id).transform();

            // Fire Transformed trigger
            ctx.trigger_handler.run_trigger(
                crate::trigger::TriggerType::Transformed,
                crate::event::RunParams {
                    card: Some(card_id),
                    ..Default::default()
                },
                false,
            );

            // Re-scan active triggers so the new face's trigger list takes effect.
            ctx.trigger_handler.reset_active_triggers(ctx.game);
        }
        Some(SpellAbilityMode::Flip) => {
            // Toggle the flipped state.
            let card = ctx.game.card_mut(card_id);
            card.set_flipped(!card.flipped);
        }
        Some(SpellAbilityMode::TurnFaceUp) => {
            if crate::replacement::replacement_handler::cant_happen_check(
                ctx.game,
                &ReplacementEvent::TurnFaceUp { card: card_id },
            ) {
                return;
            }
            let card = ctx.game.card_mut(card_id);
            if card.face_down {
                card.set_face_down(false);
                // Restore original P/T by clearing the face-down overrides
                card.set_static_set_pt(None, None);

                // Remove the synthetic morph turn-face-up ability
                card.activated_abilities.retain(|ab| !ab.is_turn_face_up());

                // Megamorph: add a +1/+1 counter when turning face-up
                if sa.param_is_true(keys::MEGA) {
                    ctx.add_counter(
                        card_id,
                        &crate::card::CounterType::P1P1,
                        1,
                        sa,
                        crate::event::RunParams::default(),
                    );
                }

                // Keep in sync with Card.turnFaceUp: the replacement runs on the face-up card.
                let mut faceup_event = ReplacementEvent::TurnFaceUp { card: card_id };
                crate::replacement::replacement_handler::apply_replacements_with_agents(
                    ctx.game,
                    ctx.agents,
                    &mut faceup_event,
                );

                // Fire TurnFaceUp trigger
                ctx.trigger_handler.run_trigger(
                    crate::trigger::TriggerType::TurnFaceUp,
                    crate::event::RunParams {
                        card: Some(card_id),
                        ..Default::default()
                    },
                    false,
                );

                // Re-scan active triggers for the revealed card
                ctx.trigger_handler.reset_active_triggers(ctx.game);
            }
        }
        Some(SpellAbilityMode::TurnFaceDown) => {
            let card = ctx.game.card_mut(card_id);
            if !card.face_down {
                card.set_face_down(true);
            }
        }
        _ => {
            let err = crate::ability::IllegalAbilityException::new(format!(
                "Unknown SetState mode: {:?}",
                mode.map(ToString::to_string)
            ));
            eprintln!("{err}");
        }
    }
}

/// Check if a card matches a comma-separated type filter (OR semantics).
/// E.g. `"Card.Instant,Card.Sorcery"` → true if the card is an Instant or Sorcery.
fn matches_type_filter(ctx: &EffectContext, card_id: CardId, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    for part in filter.split(',') {
        let type_name = part.trim().strip_prefix("Card.").unwrap_or(part.trim());
        let card = ctx.game.card(card_id);
        if card
            .type_line
            .core_types
            .iter()
            .any(|t| t.name().eq_ignore_ascii_case(type_name))
        {
            return true;
        }
    }
    false
}

/// Evaluate a `ConditionCompare` expression (e.g. `"EQ1"`, `"GT0"`) against a count.
fn evaluate_compare(compare: &str, count: usize) -> bool {
    if compare.len() < 3 {
        return true;
    }
    let op = &compare[..2];
    let num: usize = compare[2..].parse().unwrap_or(0);
    match op {
        "EQ" => count == num,
        "GT" => count > num,
        "GE" => count >= num,
        "LT" => count < num,
        "LE" => count <= num,
        "NE" => count != num,
        _ => true,
    }
}
