use forge_foundation::ZoneType;

use super::helpers::matches_valid_cards_for_sa;
use super::EffectContext;
use crate::agent::types::GameEntity;
use crate::ids::{CardId, PlayerId};
use crate::parsing::keys;
use crate::player::player_controller::PlayerController;
use crate::replacement::replacement_handler::{apply_replacements, ReplacementEvent};
use crate::replacement::ReplacementResult;
use crate::spellability::SpellAbility;

/// SP$ Attach / AB$ Attach — attach source Equipment/Aura to target creature.
///
/// Mirrors Java's `AttachEffect.resolve()`.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `AttachEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(AttachEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(source) = sa.source else {
        return;
    };
    let chooser = sa
        .chooser()
        .and_then(|defined| {
            crate::ability::ability_utils::resolve_defined_players_with_sa(
                defined,
                sa,
                sa.activating_player,
                ctx.game,
            )
            .into_iter()
            .next()
        })
        .unwrap_or(sa.activating_player);

    let attachments: Vec<CardId> = if let Some(object) = sa.ir.object_text.as_deref() {
        crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(ctx.game, sa, object)
    } else if let Some(choices) = sa.ir.choices.as_deref() {
        let candidates: Vec<GameEntity> = choice_cards(ctx, sa, choices)
            .into_iter()
            .map(GameEntity::Card)
            .collect();
        match choose_single_entity(ctx, chooser, &candidates) {
            Some(GameEntity::Card(card)) => vec![card],
            _ => return,
        }
    } else {
        vec![source]
    };
    if attachments.is_empty() {
        return;
    }

    let attach_to = if let (Some(_), Some(choices)) =
        (sa.ir.object_text.as_deref(), sa.ir.choices.as_deref())
    {
        let mut card_choices = choice_cards(ctx, sa, choices);
        for &attachment in &attachments {
            if sa.param_is_true(keys::MOVE) {
                if let Some(host) = ctx.game.card(attachment).attached_to {
                    card_choices.retain(|&c| c != host);
                }
            }
            card_choices.retain(|&c| {
                crate::card::card_predicates::can_be_attached(ctx.game, c, attachment)
            });
        }
        let candidates: Vec<GameEntity> = card_choices.into_iter().map(GameEntity::Card).collect();
        choose_single_entity(ctx, chooser, &candidates)
    } else {
        let targets = defined_entities_or_targeted(ctx, sa, source);
        if targets.is_empty() {
            return;
        }
        choose_single_entity(ctx, chooser, &targets)
    };
    let Some(GameEntity::Card(target)) = attach_to else {
        return;
    };

    let attachments = ctx.game.order_cards_by_their_owners(
        attachments,
        ZoneType::Battlefield,
        &mut Some(ctx.agents),
    );
    for attachment in attachments {
        if sa.ir.optional {
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let message = format!(
                "Do you want to attach {} to {}?",
                ctx.game.card(attachment).card_name,
                ctx.game.card(target).card_name
            );
            if !ctx.agents[chooser.index()].confirm_action(
                chooser,
                None,
                &message,
                &[],
                Some(attachment),
                Some(crate::ability::api_type::ApiType::Attach),
            ) {
                continue;
            }
        }
        if attach_to_entity(ctx, attachment, target) && sa.param_is_true(keys::REMEMBER_ATTACHED) {
            ctx.game.card_mut(source).add_remembered_card(attachment);
        }
    }
}

fn choice_cards(ctx: &EffectContext, sa: &SpellAbility, choices: &str) -> Vec<CardId> {
    let zone = sa.ir.choice_zone.unwrap_or(ZoneType::Battlefield);
    ctx.game
        .players
        .iter()
        .flat_map(|p| ctx.game.cards_in_zone(zone, p.id).iter().copied())
        .filter(|&cid| matches_valid_cards_for_sa(ctx.game, sa, ctx.game.card(cid), None, choices))
        .collect()
}

fn defined_entities_or_targeted(
    ctx: &EffectContext,
    sa: &SpellAbility,
    source: CardId,
) -> Vec<GameEntity> {
    let _ = source;
    if sa.defined().is_none() {
        let mut targets: Vec<GameEntity> = Vec::new();
        if let Some(target_card) = sa.target_chosen.target_card {
            targets.push(GameEntity::Card(target_card));
        }
        if let Some(target_player) = sa.target_chosen.target_player {
            targets.push(GameEntity::Player(target_player));
        }
        return targets;
    }
    let (players, cards) = crate::ability::spell_ability_effect::get_target_entities(ctx.game, sa);
    cards
        .into_iter()
        .map(GameEntity::Card)
        .chain(players.into_iter().map(GameEntity::Player))
        .collect()
}

fn choose_single_entity(
    ctx: &mut EffectContext,
    chooser: PlayerId,
    candidates: &[GameEntity],
) -> Option<GameEntity> {
    let agent = ctx.agents[chooser.index()].as_mut();
    let mut controller = PlayerController::new(ctx.game, chooser, agent);
    controller.snapshot_state(ctx.mana_pools);
    controller.choose_single_entity_for_effect(candidates)
}

fn attach_to_entity(ctx: &mut EffectContext, attachment: CardId, target: CardId) -> bool {
    if ctx.game.card(attachment).zone != ZoneType::Battlefield
        || ctx.game.card(target).zone != ZoneType::Battlefield
    {
        return false;
    }
    if crate::staticability::static_ability_cant_attach::cant_attach(
        &ctx.game.cards,
        ctx.game.card(attachment),
        ctx.game.card(target),
        false,
    ) {
        return false;
    }

    let mut event = ReplacementEvent::Attached {
        card: attachment,
        target,
    };
    let result = apply_replacements(ctx.game, &mut event);
    if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
        return false;
    }

    ctx.game.attach_to(attachment, target);

    ctx.trigger_handler.run_trigger(
        crate::trigger::TriggerType::Attached,
        crate::event::RunParams {
            source_card: Some(attachment),
            card: Some(target),
            ..Default::default()
        },
        false,
    );
    true
}
