//! ChangeTargets effect — redirect a spell or ability's targets.
//!
//! Ported from Java's `ChangeTargetsEffect.java`.
//! Change the target(s) of target spell or ability.

use forge_foundation::ZoneType;

use super::EffectContext;
use crate::agent::{GameEntity, GameObject};
use crate::event::RunParams;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::spellability::{SpellAbility, TargetChoices};
use crate::trigger::TriggerType;

/// Configure the spell ability during construction.
/// Mirrors Java `ChangeTargetsEffect.buildSpellAbility` — sets the target zone
/// to Stack so that the ability targets spells on the stack.
pub fn build_spell_ability(sa: &mut SpellAbility) {
    if sa.uses_targeting() {
        if let Some(ref mut tr) = sa.target_restrictions {
            tr.tgt_zone = vec![ZoneType::Stack];
        }
    }
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ChangeTargetsEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ChangeTargetsEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let activator = sa.activating_player;
    let random = sa.param_is_true(keys::RANDOM_TARGET);
    let chooser = sa
        .ir
        .chooser
        .as_deref()
        .and_then(|defined| {
            crate::ability::ability_utils::resolve_defined_players_with_sa(
                defined, sa, activator, ctx.game,
            )
            .into_iter()
            .next()
        })
        .unwrap_or(activator);

    for stack_id in get_target_spell_instances(ctx, sa) {
        let Some(mut changing) = ctx
            .game
            .stack
            .find_by_id(stack_id)
            .map(|si| si.spell_ability.clone())
        else {
            continue;
        };
        let mut distinct_objects = Vec::new();

        if sa.ir.optional {
            let card_name = changing
                .source
                .map(|cid| ctx.game.card(cid).card_name.clone())
                .unwrap_or_default();
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            if !ctx.agents[chooser.index()].confirm_action(
                chooser,
                None,
                &format!("Do you want to change targets for {card_name}?"),
                &[],
                sa.source,
                sa.api,
            ) {
                continue;
            }
        }

        if crate::parsing::raw_has_key(&sa.ability_text, "ChangeSingleTarget") {
            let mut all_targets = Vec::new();
            for (depth, node) in chain(&changing).enumerate() {
                if node.uses_targeting() {
                    for card in node.target_chosen.all_target_cards() {
                        all_targets.push((depth, GameObject::Entity(GameEntity::Card(card))));
                    }
                    for player in node.target_chosen.all_target_players() {
                        all_targets.push((depth, GameObject::Entity(GameEntity::Player(player))));
                    }
                    if let Some(stack_id) = node.target_chosen.target_stack_entry {
                        all_targets.push((depth, GameObject::Spell(stack_id)));
                    }
                }
            }
            if all_targets.is_empty() {
                return;
            }
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let Some(pick) =
                ctx.agents[chooser.index()].choose_target(chooser, sa, &all_targets, ctx.game)
            else {
                continue;
            };
            let (depth, old_target) = all_targets[pick];
            let new_target = defined_magnet(ctx, sa);
            let replace_in = node_mut(&mut changing, depth);
            let old_target_block = replace_in.target_chosen.clone();
            if let Some(new_target) = new_target.filter(|&new_target| {
                !old_target_block.contains(new_target)
                    && replace_in.can_target(new_target, ctx.game)
            }) {
                match old_target {
                    GameObject::Entity(GameEntity::Card(old)) => replace_in
                        .target_chosen
                        .replace_target_card(old, new_target),
                    GameObject::Entity(GameEntity::Player(old)) => {
                        remove_player(&mut replace_in.target_chosen, old);
                        add_card(&mut replace_in.target_chosen, new_target);
                    }
                    GameObject::Spell(_) => {
                        replace_in.target_chosen.target_stack_entry = None;
                        add_card(&mut replace_in.target_chosen, new_target);
                    }
                }
                if replace_in.target_chosen.target_card == Some(new_target) {
                    replace_in.target_chosen.target_card_zone_timestamp =
                        Some(ctx.game.card(new_target).zone_timestamp);
                }
                update_target(&old_target_block, replace_in, &mut distinct_objects);
            }
        } else if random {
            let candidates: Vec<CardId> = ctx
                .game
                .cards
                .iter()
                .filter(|c| c.zone == ZoneType::Battlefield)
                .map(|c| c.id)
                .collect();
            if candidates.is_empty() {
                return;
            }
            let idx = ctx.rng.next_int(candidates.len() as i32) as usize % candidates.len();
            if let Some(host) = changing.source {
                ctx.game
                    .card_mut(host)
                    .set_s_var("RedirectedTarget", format!("{}", candidates[idx].0));
            }
            return;
        } else {
            for depth in 0..chain(&changing).count() {
                let node = node_mut(&mut changing, depth);
                if !node.uses_targeting() {
                    continue;
                }
                if sa.ir.defined_magnet_text.is_some() {
                    let Some(new_target) = defined_magnet(ctx, sa)
                        .filter(|&new_target| node.can_target(new_target, ctx.game))
                    else {
                        continue;
                    };
                    let div: i32 = node.target_chosen.divided_map.values().sum();
                    let old_target = std::mem::take(&mut node.target_chosen);
                    add_card(&mut node.target_chosen, new_target);
                    node.target_chosen.target_card_zone_timestamp =
                        Some(ctx.game.card(new_target).zone_timestamp);
                    if crate::parsing::raw_has_key(&node.ability_text, "DividedAsYouChoose") {
                        node.add_divided_allocation(new_target, div);
                    }
                    update_target(&old_target, node, &mut distinct_objects);
                } else {
                    let old_target = node.target_chosen.clone();
                    ctx.agents[chooser.index()].choose_new_targets_for(
                        node,
                        ctx.game,
                        ctx.mana_pools,
                        false,
                    );
                    update_target(&old_target, node, &mut distinct_objects);
                }
            }
        }

        if let Some(si) = ctx.game.stack.iter_mut().find(|si| si.id == stack_id) {
            si.spell_ability = changing.clone();
        }
        run_becomes_target_triggers(ctx, sa, &changing, &distinct_objects);
    }
}

fn get_target_spell_instances(ctx: &EffectContext, sa: &SpellAbility) -> Vec<u32> {
    if sa.uses_targeting() {
        sa.target_chosen.target_stack_entry.into_iter().collect()
    } else {
        sa.defined()
            .map(|defined| {
                crate::ability::ability_utils::get_defined_spell_abilities(defined, sa, ctx.game)
            })
            .unwrap_or_default()
            .iter()
            .filter_map(|tgt_sa| {
                ctx.game
                    .stack
                    .get_instance_matching_spell_ability_id(tgt_sa.id)
                    .map(|si| si.id)
            })
            .collect()
    }
}

fn defined_magnet(ctx: &EffectContext, sa: &SpellAbility) -> Option<CardId> {
    let defined = sa.ir.defined_magnet_text.as_deref()?;
    crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(ctx.game, sa, defined)
        .first()
        .copied()
}

fn chain(sa: &SpellAbility) -> impl Iterator<Item = &SpellAbility> {
    std::iter::successors(Some(sa), |node| node.sub_ability.as_deref())
}

fn node_mut(sa: &mut SpellAbility, depth: usize) -> &mut SpellAbility {
    let mut node = sa;
    for _ in 0..depth {
        node = node
            .sub_ability
            .as_deref_mut()
            .expect("depth counted along the sub-ability chain");
    }
    node
}

fn add_card(targets: &mut TargetChoices, card: CardId) {
    if targets.target_card.is_none() {
        targets.target_card = Some(card);
    } else {
        targets.divided_map.insert(card, 0);
    }
}

fn remove_player(targets: &mut TargetChoices, player: crate::ids::PlayerId) {
    targets.additional_target_players.retain(|&p| p != player);
    if targets.target_player == Some(player) {
        targets.target_player = if targets.additional_target_players.is_empty() {
            None
        } else {
            Some(targets.additional_target_players.remove(0))
        };
    }
}

fn update_target(old: &TargetChoices, node: &SpellAbility, distinct_objects: &mut Vec<GameEntity>) {
    let old_players = old.all_target_players();
    let new_targets = node
        .target_chosen
        .all_target_cards()
        .into_iter()
        .filter(|&card| !old.contains(card))
        .map(GameEntity::Card)
        .chain(
            node.target_chosen
                .all_target_players()
                .into_iter()
                .filter(|player| !old_players.contains(player))
                .map(GameEntity::Player),
        );
    for target in new_targets {
        if !distinct_objects.contains(&target) {
            distinct_objects.push(target);
        }
    }
}

fn run_becomes_target_triggers(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    tgt_sa: &SpellAbility,
    distinct_objects: &[GameEntity],
) {
    let activator = tgt_sa.activating_player;
    for &target in distinct_objects {
        let params = match target {
            GameEntity::Card(card) => {
                let first_time = !ctx.game.card(card).has_become_target_this_turn();
                let valiant = ctx.game.card(card).is_valiant(activator);
                ctx.game.card_mut(card).add_target_from_this_turn(activator);
                RunParams {
                    card: Some(card),
                    target_card: Some(card),
                    cards: Some(vec![card]),
                    source_sa: Some(tgt_sa.clone()),
                    first_time: Some(first_time),
                    valiant: Some(valiant),
                    ..Default::default()
                }
            }
            GameEntity::Player(player) => RunParams {
                player: Some(player),
                target_player: Some(player),
                source_sa: Some(tgt_sa.clone()),
                ..Default::default()
            },
        };
        ctx.trigger_handler
            .run_trigger(TriggerType::BecomesTarget, params, false);
    }
    if !distinct_objects.is_empty() {
        let cards = distinct_objects
            .iter()
            .filter_map(|target| match target {
                GameEntity::Card(card) => Some(*card),
                GameEntity::Player(_) => None,
            })
            .collect();
        ctx.trigger_handler.run_trigger(
            TriggerType::BecomesTargetOnce,
            RunParams {
                cards: Some(cards),
                source_sa: Some(tgt_sa.clone()),
                cause_card: sa.source,
                ..Default::default()
            },
            false,
        );
    }
}
