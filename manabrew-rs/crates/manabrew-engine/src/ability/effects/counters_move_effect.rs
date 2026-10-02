//! Mirrors Java's `CountersMoveEffect`, which `MoveCounter` runs.

use forge_foundation::ZoneType;

use super::{parse_counter_type, EffectContext};
use crate::agent::{DecisionContext, GameEntity};
use crate::card::CounterType;
use crate::event::RunParams;
use crate::game_entity_counter_table::GameEntityCounterTable;
use crate::ids::CardId;
use crate::spellability::SpellAbility;

#[manabrew_engine_macros::spell_effect(CountersMoveEffect)]
fn resolve(ctx: &mut EffectContext, sa: &SpellAbility) {
    let Some(counter_name) = crate::parsing::raw_get(&sa.ability_text, "CounterType") else {
        return;
    };
    let counter_name = counter_name.trim();
    let counter_num = crate::parsing::raw_get(&sa.ability_text, "CounterNum")
        .map(str::trim)
        .unwrap_or("1");
    let activator = sa.activating_player;
    let c_type =
        (counter_name != "Any" && counter_name != "All").then(|| parse_counter_type(counter_name));

    let mut table = GameEntityCounterTable::default();

    if let Some(valid_source) = crate::parsing::raw_get(&sa.ability_text, "ValidSource") {
        let Some(&dest) =
            crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa)
                .first()
        else {
            return;
        };
        if !is_expected_state(ctx, sa, dest) {
            return;
        }
        let mut src_cards = valid_battlefield_cards(ctx, sa, valid_source);
        if counter_name == "All" {
            if counter_num == "Any" {
                src_cards.retain(|&cid| ctx.game.card(cid).sum_all_counters() > 0);
                src_cards = choose_cards(ctx, sa, &src_cards);
            }
        } else {
            let c_type = c_type.clone().expect("a named counter type");
            if !can_receive_counters(ctx, dest, &c_type) {
                return;
            }
            src_cards.retain(|&cid| ctx.game.card(cid).counter_count(&c_type) > 0);
            if counter_num == "Any" {
                src_cards = choose_cards(ctx, sa, &src_cards);
            }
        }

        let mut counters_to_add: Vec<(CounterType, i32)> = Vec::new();
        for src in src_cards {
            if counter_name == "All" {
                let types: Vec<CounterType> = ctx.game.card(src).counters.keys().cloned().collect();
                for counter_type in types {
                    remove_counter(
                        ctx,
                        sa,
                        src,
                        dest,
                        &counter_type,
                        counter_num,
                        &mut counters_to_add,
                    );
                }
            } else {
                let c_type = c_type.clone().expect("a named counter type");
                remove_counter(
                    ctx,
                    sa,
                    src,
                    dest,
                    &c_type,
                    counter_num,
                    &mut counters_to_add,
                );
            }
        }
        for (counter_type, amount) in counters_to_add {
            table.put(
                Some(activator),
                GameEntity::Card(dest),
                counter_type,
                amount,
            );
        }
    } else if let Some(valid_defined) = crate::parsing::raw_get(&sa.ability_text, "ValidDefined") {
        let Some(&source) =
            crate::ability::spell_ability_effect::get_defined_cards_or_targeted_param(
                ctx.game, sa, "Source",
            )
            .first()
        else {
            return;
        };
        let c_type = c_type.expect("a named counter type");
        if ctx.game.card(source).counter_count(&c_type) <= 0 {
            return;
        }
        let mut tgt_cards = valid_battlefield_cards(ctx, sa, valid_defined);
        if counter_num == "Any" {
            tgt_cards = choose_cards(ctx, sa, &tgt_cards);
        }
        for dest in tgt_cards {
            if source == dest
                || !can_receive_counters(ctx, dest, &c_type)
                || !ctx.game.card(source).can_remove_counters(&c_type)
                || !is_expected_state(ctx, sa, dest)
            {
                continue;
            }
            let max = ctx.game.card(source).counter_count(&c_type);
            let amount = ctx.agents[activator.index()]
                .choose_number(
                    DecisionContext::new(ctx.game, ctx.mana_pools),
                    activator,
                    sa.source,
                    "Put how many counters?",
                    None,
                    0,
                    max,
                )
                .unwrap_or(0)
                .clamp(0, max);
            if amount > 0 {
                super::counters_remove_effect::subtract_counter(ctx, source, &c_type, amount);
                table.put(
                    Some(activator),
                    GameEntity::Card(dest),
                    c_type.clone(),
                    amount,
                );
            }
        }
    } else {
        let mut tgt_cards =
            crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa);
        let source = if sa
            .target_restrictions
            .as_ref()
            .is_some_and(|tr| tr.get_min_targets(ctx.game, sa) == 2)
        {
            if tgt_cards.len() < 2 {
                return;
            }
            Some(tgt_cards.remove(0))
        } else {
            crate::ability::spell_ability_effect::get_defined_cards_or_targeted_param(
                ctx.game, sa, "Source",
            )
            .first()
            .copied()
        };
        let Some(source) = source else {
            return;
        };
        if ctx.game.card(source).sum_all_counters() <= 0 {
            return;
        }

        for dest in tgt_cards {
            if source == dest || !is_expected_state(ctx, sa, dest) {
                continue;
            }
            let mut counters_to_add: Vec<(CounterType, i32)> = Vec::new();
            match counter_name {
                "All" | "EachNotOn" => {
                    let types: Vec<CounterType> =
                        ctx.game.card(source).counters.keys().cloned().collect();
                    for counter_type in types {
                        if counter_name == "EachNotOn"
                            && ctx.game.card(dest).counter_count(&counter_type) > 0
                        {
                            continue;
                        }
                        remove_counter(
                            ctx,
                            sa,
                            source,
                            dest,
                            &counter_type,
                            counter_num,
                            &mut counters_to_add,
                        );
                    }
                }
                "Any" => {
                    let mut type_choices: Vec<CounterType> = ctx
                        .game
                        .card(source)
                        .counters
                        .iter()
                        .filter(|&(_, &amount)| amount > 0)
                        .map(|(counter_type, _)| counter_type.clone())
                        .filter(|counter_type| {
                            can_receive_counters(ctx, dest, counter_type)
                                && ctx.game.card(source).can_remove_counters(counter_type)
                        })
                        .collect();
                    if type_choices.is_empty() {
                        return;
                    }
                    while !type_choices.is_empty() {
                        let Some(chosen) = ctx.agents[activator.index()].choose_counter_type(
                            DecisionContext::new(ctx.game, ctx.mana_pools),
                            activator,
                            &type_choices,
                            "Select type of counters to remove",
                        ) else {
                            break;
                        };
                        remove_counter(
                            ctx,
                            sa,
                            source,
                            dest,
                            &chosen,
                            counter_num,
                            &mut counters_to_add,
                        );
                        if counter_num != "Any" {
                            break;
                        }
                        type_choices.retain(|counter_type| *counter_type != chosen);
                    }
                }
                _ => {
                    let c_type = c_type.clone().expect("a named counter type");
                    remove_counter(
                        ctx,
                        sa,
                        source,
                        dest,
                        &c_type,
                        counter_num,
                        &mut counters_to_add,
                    );
                }
            }
            for (counter_type, amount) in counters_to_add {
                table.put(
                    Some(activator),
                    GameEntity::Card(dest),
                    counter_type,
                    amount,
                );
            }
        }
    }

    table.replace_counter_effect(
        ctx.game,
        Some(ctx.trigger_handler),
        Some(ctx.agents),
        Some(sa),
        true,
        RunParams {
            source_player: Some(activator),
            cause: Some(sa.clone()),
            ..Default::default()
        },
    );
}

fn remove_counter(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    src: CardId,
    dest: CardId,
    counter_type: &CounterType,
    counter_num: &str,
    counters_to_add: &mut Vec<(CounterType, i32)>,
) {
    if src == dest
        || !can_receive_counters(ctx, dest, counter_type)
        || !ctx.game.card(src).can_remove_counters(counter_type)
    {
        return;
    }
    let max = ctx.game.card(src).counter_count(counter_type);
    if max <= 0 {
        return;
    }
    let amount = match counter_num {
        "All" => max,
        "Any" => {
            let min = i32::from(
                crate::parsing::raw_has_key(&sa.ability_text, "NonZero")
                    && counters_to_add.is_empty(),
            );
            let activator = sa.activating_player;
            ctx.agents[activator.index()]
                .choose_number(
                    DecisionContext::new(ctx.game, ctx.mana_pools),
                    activator,
                    sa.source,
                    "Take how many counters?",
                    None,
                    min,
                    max,
                )
                .unwrap_or(min)
                .clamp(min, max)
        }
        amount => max.min(crate::svar::resolve_numeric_value(ctx.game, sa, amount, 1)),
    };
    if amount > 0 {
        super::counters_remove_effect::subtract_counter(ctx, src, counter_type, amount);
        match counters_to_add
            .iter_mut()
            .find(|(ct, _)| ct == counter_type)
        {
            Some((_, total)) => *total += amount,
            None => counters_to_add.push((counter_type.clone(), amount)),
        }
    }
}

fn can_receive_counters(ctx: &EffectContext, card: CardId, counter_type: &CounterType) -> bool {
    crate::card::card_predicates::can_receive_counters(ctx.game, card, counter_type)
}

/// Java's `game.getCardState(dest).equalsWithGameTimestamp(dest)`: a card defined through an LKI
/// copy that has changed zones since is not the same object any more.
fn is_expected_state(ctx: &EffectContext, sa: &SpellAbility, card: CardId) -> bool {
    sa.trigger_object_timestamps
        .iter()
        .find(|(card_id, _)| *card_id == card)
        .is_none_or(|&(_, timestamp)| timestamp == ctx.game.card(card).zone_timestamp)
}

fn valid_battlefield_cards(ctx: &EffectContext, sa: &SpellAbility, valid: &str) -> Vec<CardId> {
    let selector = crate::parsing::cached_compiled_selector(valid);
    ctx.game
        .player_order
        .iter()
        .flat_map(|&pid| {
            ctx.game
                .cards_in_zone(ZoneType::Battlefield, pid)
                .iter()
                .copied()
        })
        .filter(|&cid| {
            crate::ability::ability_utils::matches_valid_cards_for_sa(
                ctx.game,
                sa,
                ctx.game.card(cid),
                Some(&selector),
                valid,
            )
        })
        .collect()
}

fn choose_cards(ctx: &mut EffectContext, sa: &SpellAbility, cards: &[CardId]) -> Vec<CardId> {
    let activator = sa.activating_player;
    ctx.agents[activator.index()].choose_cards_for_effect(
        DecisionContext::new(ctx.game, ctx.mana_pools),
        activator,
        cards,
        0,
        cards.len(),
    )
}
