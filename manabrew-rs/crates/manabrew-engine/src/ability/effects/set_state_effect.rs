use super::EffectContext;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::replacement::replacement_handler::{
    apply_replacements_with_agents_and_runtime, ReplacementEvent, ReplacementRuntime,
};
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
    let cards_to_transform = if let Some(filter) = sa.ir.choices.as_deref() {
        let valid_amount = sa.ir.amount.as_deref().map_or(1, |amount| {
            crate::svar::resolve_numeric_value(ctx.game, sa, amount, 1)
        });
        let min_amount = sa
            .ir
            .min_amount
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(valid_amount);
        if valid_amount <= 0 {
            return;
        }
        let mut choices = Vec::new();
        for &pid in &ctx.game.player_order {
            for &cid in ctx
                .game
                .cards_in_zone(forge_foundation::ZoneType::Battlefield, pid)
            {
                if super::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    sa.ir.choices_selector.as_ref(),
                    filter,
                ) {
                    choices.push(cid);
                }
            }
        }
        let player = sa.activating_player;
        ctx.agents[player.index()].snapshot_state(ctx.game, ctx.mana_pools);
        ctx.agents[player.index()].choose_cards_for_effect(
            player,
            &choices,
            min_amount.max(0) as usize,
            valid_amount as usize,
        )
    } else {
        crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa)
    };
    for card_id in cards_to_transform {
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
        if mode == Some(&SpellAbilityMode::Transform) && sa.source == Some(card_id) {
            if let Some(stored) = crate::parsing::raw_get(&sa.ability_text, "StoredTransform") {
                if stored.parse().ok() != Some(ctx.game.card(card_id).transform_count) {
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
            ctx.game.card_mut(card_id).transform();
            ctx.game.card_mut(card_id).transform_count += 1;

            let mut transform_event = ReplacementEvent::Transform { card: card_id };
            let mut runtime = ReplacementRuntime {
                trigger_handler: ctx.trigger_handler,
                token_templates: ctx.token_templates,
                token_art_variants: ctx.token_art_variants,
                token_fallback: ctx.token_fallback,
                edition_dates: ctx.edition_dates,
                mana_pools: ctx.mana_pools,
                rng: ctx.rng,
            };
            apply_replacements_with_agents_and_runtime(
                ctx.game,
                ctx.agents,
                &mut runtime,
                &mut transform_event,
            );

            if !crate::parsing::raw_has_key(&sa.ability_text, "ETB") {
                ctx.trigger_handler.run_trigger(
                    crate::trigger::TriggerType::Transformed,
                    crate::event::RunParams {
                        card: Some(card_id),
                        ..Default::default()
                    },
                    false,
                );
            }

            // Re-scan active triggers so the new face's trigger list takes effect.
            ctx.trigger_handler.reset_active_triggers(ctx.game);
            if sa.is_remember_changed() {
                if let Some(host) = sa.source {
                    ctx.game.card_mut(host).add_remembered_card(card_id);
                }
            }
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
                        cause: Some(sa.clone()),
                        ..Default::default()
                    },
                    false,
                );

                // Re-scan active triggers for the revealed card
                ctx.trigger_handler.reset_active_triggers(ctx.game);
                if sa.is_remember_changed() {
                    if let Some(host) = sa.source {
                        ctx.game.card_mut(host).add_remembered_card(card_id);
                    }
                }
            }
        }
        Some(SpellAbilityMode::TurnFaceDown) => {
            let card = ctx.game.card(card_id);
            if card.face_down || card.is_double_faced() {
                return;
            }
            ctx.game.run_facedown_commands(card_id, ctx.rng);
            crate::card::card_factory_util::turn_face_down_with_state(ctx.game.card_mut(card_id));
            if ["FaceDownPower", "FaceDownToughness", "FaceDownSetType"]
                .iter()
                .any(|key| crate::parsing::raw_has_key(&sa.ability_text, key))
            {
                crate::card::card_factory_util::set_face_down_state(ctx.game, card_id, sa);
            }
            ctx.trigger_handler.reset_active_triggers(ctx.game);
            if sa.is_remember_changed() {
                if let Some(host) = sa.source {
                    ctx.game.card_mut(host).add_remembered_card(card_id);
                }
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
