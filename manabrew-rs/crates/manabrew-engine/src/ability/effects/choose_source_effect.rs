use forge_foundation::ZoneType;

use super::{matches_valid_cards_for_sa, EffectContext};
use crate::agent::GameEntity;
use crate::event::AbilityValue;
use crate::ids::CardId;

/// `SP$ ChooseSource` — the activating player chooses a source (permanent/spell).
/// Stores the result for subsequent effects.
///
/// Mirrors Java's `ChooseSourceEffect.java`.
/// - `Choices$` — filter for valid sources (default: Permanent on battlefield).
///
/// # Card script examples
/// ```text
/// A:SP$ ChooseSource | Choices$ Permanent
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ChooseSourceEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ChooseSourceEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(host) = sa.source else {
        return;
    };
    let tgt_players = crate::ability::spell_ability_effect::get_target_players(ctx.game, sa);

    let player_ids = ctx.game.player_order.clone();
    let mut sources: Vec<CardId> = Vec::new();
    for &pid in &player_ids {
        sources.extend(
            ctx.game
                .cards_in_zone(ZoneType::Battlefield, pid)
                .iter()
                .copied(),
        );
    }
    let stack_entries: Vec<&crate::zone::magic_stack::StackEntry> = ctx
        .game
        .stack
        .resolving_entry()
        .into_iter()
        .chain(ctx.game.stack.iter())
        .collect();
    for entry in &stack_entries {
        sources.extend(entry.spell_ability.source);
    }
    for entry in &stack_entries {
        let si_sa = &entry.spell_ability;
        for value in si_sa
            .trigger_objects
            .values()
            .chain(si_sa.replacing_objects.values())
        {
            if let AbilityValue::Card(card) = value {
                sources.push(*card);
            }
        }
        sources.extend(si_sa.target_chosen.target_card);
    }
    for &pid in &player_ids {
        sources.extend(
            ctx.game
                .cards_in_zone(ZoneType::Command, pid)
                .iter()
                .copied()
                .filter(|&cid| !ctx.game.card(cid).face_down),
        );
    }
    let mut seen = Vec::new();
    sources.retain(|cid| {
        if seen.contains(cid) {
            false
        } else {
            seen.push(*cid);
            true
        }
    });

    if let Some(choices) = sa.ir.choices.as_deref() {
        sources.retain(|&cid| {
            matches_valid_cards_for_sa(
                ctx.game,
                sa,
                ctx.game.card(cid),
                sa.ir.choices_selector.as_ref(),
                choices,
            )
        });
    }
    if crate::parsing::raw_has_key(&sa.ability_text, "TargetControls") {
        if let Some(&first) = tgt_players.first() {
            sources.retain(|&cid| ctx.game.card(cid).controller == first);
        }
    }
    if sources.is_empty() {
        return;
    }

    let amount = sa.ir.amount.as_deref().map_or(1, |raw| {
        crate::svar::resolve_numeric_value(ctx.game, sa, raw, 1)
    });

    for p in tgt_players {
        if ctx.game.player(p).has_lost {
            continue;
        }
        let mut options: Vec<GameEntity> = sources.iter().copied().map(GameEntity::Card).collect();
        let mut chosen = Vec::new();
        for _ in 0..amount {
            ctx.agents[p.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let Some(GameEntity::Card(card)) =
                ctx.agents[p.index()].choose_single_entity_for_effect(p, &options, false)
            else {
                break;
            };
            chosen.push(card);
            options.retain(|entity| *entity != GameEntity::Card(card));
        }
        ctx.game.card_mut(host).set_chosen_cards(chosen.clone());
        if sa.ir.remember_chosen {
            for &cid in &chosen {
                ctx.game.card_mut(host).add_remembered_card(cid);
            }
        }
    }
}
