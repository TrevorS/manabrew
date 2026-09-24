use super::EffectContext;
use crate::event::RunParams;
use crate::replacement::replacement_handler::{apply_replacements, ReplacementEvent};
use crate::replacement::ReplacementResult;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

/// Configure the spell ability during construction.
/// Mirrors Java `CopySpellAbilityEffect.buildSpellAbility` — sets the target zone
/// to Stack so the ability targets spells on the stack.
pub fn build_spell_ability(sa: &mut SpellAbility) {
    if sa.uses_targeting() {
        if let Some(ref mut tr) = sa.target_restrictions {
            tr.tgt_zone = vec![forge_foundation::ZoneType::Stack];
        }
    }
}

/// `SP$ CopySpellAbility` — copy the top spell on the stack.
///
/// Mirrors Java's `CopySpellAbilityEffect.java` (basic version).
/// Creates a clone of the topmost spell on the stack with the same targets.
/// Full retargeting support deferred.
///
/// # Card script examples
/// ```text
/// A:SP$ CopySpellAbility | Defined$ TopStack
/// A:SP$ CopySpellAbility | Defined$ TriggeredSpellAbility
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `CopySpellAbilityEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(CopySpellAbilityEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let controller = sa.activating_player;

    // Run CopySpell replacement effects before copying.
    let mut event = ReplacementEvent::CopySpell {
        player: controller,
        count: 1,
    };
    let result = apply_replacements(ctx.game, &mut event);
    if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
        return;
    }

    // Java `getTargetSpells`: the targeted spells when the ability targets, else Defined$.
    let originals: Vec<crate::spellability::SpellAbility> = if sa.uses_targeting() {
        sa.target_chosen
            .target_stack_entry
            .and_then(|id| ctx.game.stack.iter().find(|entry| entry.id == id))
            .map(|entry| entry.spell_ability.clone())
            .into_iter()
            .collect()
    } else if let Some(defined) = sa.defined() {
        crate::ability::ability_utils::get_defined_spell_abilities(defined, sa, ctx.game)
    } else {
        let stack_entries: Vec<_> = ctx.game.stack.iter().collect();
        stack_entries
            .iter()
            .rev()
            .find_map(|entry| {
                if Some(entry.id) != sa.ir.stack_id {
                    Some(entry.spell_ability.clone())
                } else {
                    None
                }
            })
            .into_iter()
            .collect()
    };
    let originals: Vec<_> = originals
        .into_iter()
        .filter(|spell| {
            !crate::card::card_factory::spell_ability_cant_be_copied(&ctx.game.cards, spell)
        })
        .collect();
    let amount = super::resolve_numeric_svar(ctx.game, sa, "Amount", 1);
    if originals.is_empty() || amount <= 0 {
        return;
    }
    let controllers = match crate::parsing::raw_get(&sa.ability_text, "Controller") {
        Some(defined) => crate::ability::ability_utils::resolve_defined_players_with_sa(
            defined, sa, controller, ctx.game,
        ),
        None => vec![controller],
    };

    let optional = crate::parsing::raw_has_key(&sa.ability_text, "Optional");
    for controller in controllers {
        for original in &originals {
            if optional {
                let name = original
                    .source
                    .map(|cid| ctx.game.card(cid).card_name.clone())
                    .unwrap_or_default();
                ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
                if !ctx.agents[controller.index()].confirm_action(
                    controller,
                    None,
                    &format!("Do you want to copy {name}?"),
                    &[],
                    sa.source,
                    sa.api,
                ) {
                    continue;
                }
            }
            for _ in 0..amount {
                push_copy(ctx, sa, original, controller);
            }
        }
    }
}

fn push_copy(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    original: &crate::spellability::SpellAbility,
    controller: crate::ids::PlayerId,
) {
    let copy = crate::card::card_factory::copy_spell_ability_and_possibly_host(
        ctx.game, sa, original, controller,
    );

    // Push the copy onto the stack (it will resolve like a normal spell)
    let copy_entry = crate::spellability::StackEntry {
        id: 0, // will be assigned by push()
        spell_ability: copy,
        is_pending_cast: false,
        is_creature_spell: original.is_spell
            && original
                .source
                .is_some_and(|cid| ctx.game.card(cid).is_creature()),
        is_permanent_spell: original.is_spell
            && original
                .source
                .is_some_and(|cid| ctx.game.card(cid).is_permanent()),
        cast_from_zone: None,
        optional_trigger_decider: None,
        optional_trigger_description: None,
        optional_trigger_source_name: None,
    };

    let trigger_sa = copy_entry.spell_ability.clone();
    ctx.game.stack.push(copy_entry);
    if !trigger_sa.is_trigger {
        ctx.game.turn.priority_player = controller;
    }
    if let Some(source_id) = trigger_sa.source {
        ctx.trigger_handler.run_trigger(
            TriggerType::SpellCopied,
            RunParams {
                spell_card: Some(source_id),
                spell_controller: Some(controller),
                source_sa: Some(trigger_sa.clone()),
                ..Default::default()
            },
            false,
        );
        super::emit_targeting_triggers(ctx, source_id, &trigger_sa);
    }
}
