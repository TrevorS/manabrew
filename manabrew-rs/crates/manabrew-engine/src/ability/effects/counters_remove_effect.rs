use forge_foundation::ZoneType;

use super::{parse_counter_type, EffectContext};
use crate::card::CounterType;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::replacement::replacement_handler::{apply_replacements, ReplacementEvent};
use crate::replacement::ReplacementResult;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

#[manabrew_engine_macros::spell_effect(CountersRemoveEffect)]
fn resolve(ctx: &mut EffectContext, sa: &SpellAbility) {
    let Some(source) = sa.source else {
        return;
    };
    let activator = sa.activating_player;
    let type_text = sa.ir.counter_type_text.as_deref().unwrap_or("P1P1");
    let num = sa.ir.counter_num_text.as_deref().unwrap_or("1");

    let mut cnt_to_remove = 0;
    if num != "All" && num != "Any" {
        cnt_to_remove = crate::svar::resolve_numeric_svar(ctx.game, sa, keys::COUNTER_NUM, 1);
    }

    if sa.ir.optional {
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[activator.index()].confirm_action(
            activator,
            None,
            "Remove counters?",
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }

    let counter_type =
        (type_text != "Any" && type_text != "All").then(|| parse_counter_type(type_text));

    let remember_removed = crate::parsing::raw_get(&sa.ability_text, "RememberRemoved").is_some();
    let remember_amount = sa.ir.remember_amount;

    let mut total_removed = 0;
    let mut src_cards = if let Some(choices) = sa.ir.choices.as_deref() {
        let choice_zone = sa.ir.choice_zone.unwrap_or(ZoneType::Battlefield);
        let selector = crate::parsing::cached_compiled_selector(choices);
        let players: Vec<_> = ctx.game.players.iter().map(|p| p.id).collect();
        players
            .into_iter()
            .flat_map(|pid| ctx.game.cards_in_zone(choice_zone, pid).to_vec())
            .filter(|&cid| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    Some(&selector),
                    choices,
                )
            })
            .collect()
    } else {
        crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa)
    };

    if sa.ir.choices.is_some() {
        let mut min = 1;
        let mut max = 1;
        if crate::parsing::raw_get(&sa.ability_text, keys::CHOICE_OPTIONAL).is_some() {
            min = 0;
            max = src_cards.len();
        }
        if let Some(choice_num) = crate::parsing::raw_get(&sa.ability_text, "ChoiceNum") {
            let n = crate::svar::resolve_numeric_value(ctx.game, sa, choice_num, 0).max(0) as usize;
            min = n;
            max = n;
        }
        if src_cards.len() < min {
            return;
        }
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        src_cards =
            ctx.agents[activator.index()].choose_cards_for_effect(activator, &src_cards, min, max);
    }

    for card_id in src_cards {
        let zone = ctx.game.card(card_id).zone;
        if type_text == "All" {
            let counters: Vec<(CounterType, i32)> = ctx
                .game
                .card(card_id)
                .counters
                .iter()
                .map(|(ct, count)| (ct.clone(), *count))
                .collect();
            for (ct, count) in counters {
                total_removed += subtract_counter(ctx, card_id, &ct, count);
            }
        } else if type_text == "Any" {
            total_removed += remove_any_type(ctx, sa, card_id, cnt_to_remove, remember_removed);
        } else {
            let counter_type = counter_type.as_ref().expect("typed counter");
            if !ctx.game.card(card_id).can_remove_counters(counter_type) {
                continue;
            }

            let mut remove_from_card = cnt_to_remove;
            if num == "All" || num == "Any" {
                remove_from_card = ctx.game.card(card_id).counter_count(counter_type);
            } else {
                if crate::parsing::raw_get(&sa.ability_text, "CounterNumShared").is_some() {
                    remove_from_card -= total_removed;
                    if remove_from_card < 1 {
                        break;
                    }
                }
                remove_from_card =
                    remove_from_card.min(ctx.game.card(card_id).counter_count(counter_type));
            }

            if (zone == ZoneType::Battlefield || zone == ZoneType::Exile)
                && (sa.ir.up_to || num == "Any")
            {
                ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
                remove_from_card = ctx.agents[activator.index()]
                    .choose_number(
                        activator,
                        sa.source,
                        "Select the number of counters to remove",
                        None,
                        0,
                        remove_from_card,
                    )
                    .unwrap_or(0);
            }
            if remove_from_card > 0 {
                let removed = subtract_counter(ctx, card_id, counter_type, remove_from_card);
                if remember_removed {
                    for _ in 0..removed {
                        ctx.game
                            .card_mut(source)
                            .remembered_counters
                            .push(counter_type.clone());
                    }
                }
                total_removed += removed;
            }
        }
    }

    if total_removed > 0 && remember_amount {
        ctx.game.card_mut(source).add_remembered_cmc(total_removed);
    }
}

fn remove_any_type(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    card_id: CardId,
    mut cnt_to_remove: i32,
    remember_removed: bool,
) -> i32 {
    let mut removed = 0;
    let mut up_to = sa.ir.up_to;
    if sa.ir.counter_num_text.as_deref() == Some("Any") {
        cnt_to_remove = i32::MAX;
        up_to = true;
    }
    let activator = sa.activating_player;
    let Some(source) = sa.source else {
        return 0;
    };
    let mut tgt_counters: Vec<(CounterType, i32)> = ctx
        .game
        .card(card_id)
        .counters
        .iter()
        .filter(|(ct, _)| ctx.game.card(card_id).can_remove_counters(ct))
        .map(|(ct, count)| (ct.clone(), *count))
        .collect();

    while cnt_to_remove > 0 && !tgt_counters.is_empty() {
        let options: Vec<CounterType> = tgt_counters.iter().map(|(ct, _)| ct.clone()).collect();
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let Some(chosen_type) = ctx.agents[activator.index()].choose_counter_type(
            activator,
            &options,
            "Select type of counters to remove",
        ) else {
            break;
        };
        let type_count = tgt_counters
            .iter()
            .find(|(ct, _)| *ct == chosen_type)
            .map_or(0, |(_, count)| *count);
        let max = cnt_to_remove.min(type_count);
        if let Some(entry) = tgt_counters.iter_mut().find(|(ct, _)| *ct == chosen_type) {
            entry.1 -= 1;
        }
        tgt_counters.retain(|(_, count)| *count > 0);
        let remaining: i32 = tgt_counters.iter().map(|(_, count)| *count).sum();
        let min = if up_to { 0 } else { (max - remaining).max(1) };
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let chosen_amount = ctx.agents[activator.index()]
            .choose_number(
                activator,
                sa.source,
                "Select the number of counters to remove",
                None,
                min,
                max,
            )
            .unwrap_or(min);

        if chosen_amount > 0 {
            let actual = subtract_counter(ctx, card_id, &chosen_type, chosen_amount);
            removed += actual;
            if remember_removed {
                for _ in 0..actual {
                    ctx.game
                        .card_mut(source)
                        .remembered_counters
                        .push(chosen_type.clone());
                }
            }
            cnt_to_remove -= chosen_amount;
        } else if up_to {
            break;
        }
    }
    removed
}

fn subtract_counter(
    ctx: &mut EffectContext,
    card_id: CardId,
    counter_type: &CounterType,
    count: i32,
) -> i32 {
    let mut event = ReplacementEvent::RemoveCounter {
        target: card_id,
        counter_type: counter_type.clone(),
        count,
    };
    let result = apply_replacements(ctx.game, &mut event);
    if result == ReplacementResult::Skipped || result == ReplacementResult::Replaced {
        return 0;
    }
    let count = if let ReplacementEvent::RemoveCounter { count, .. } = event {
        count
    } else {
        count
    };
    let actual = count.min(ctx.game.card(card_id).counter_count(counter_type));
    if actual <= 0 {
        return 0;
    }
    ctx.game
        .card_mut(card_id)
        .remove_counter(counter_type, actual);
    ctx.trigger_handler.run_trigger(
        TriggerType::CounterRemoved,
        RunParams {
            card: Some(card_id),
            counter_type: Some(format!("{counter_type:?}")),
            counter_amount: Some(actual),
            ..Default::default()
        },
        false,
    );
    actual
}
