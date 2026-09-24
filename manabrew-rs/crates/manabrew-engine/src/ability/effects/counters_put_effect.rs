use forge_foundation::ZoneType;

use super::{matches_valid_cards_for_sa, parse_counter_type, resolve_numeric_svar, EffectContext};
use crate::ability::ability_ir::DefinedRef;
use crate::agent::GameEntity;
use crate::event::RunParams;
use crate::game_entity_counter_table::GameEntityCounterTable;
use crate::parsing::keys;
use crate::spellability::SpellAbility;
use crate::svar::resolve_numeric_value;
use crate::trigger::TriggerType;

pub fn build_spell_ability(sa: &mut crate::spellability::SpellAbility) {
    let Some(n) = sa.ir.adapt.clone().or_else(|| sa.ir.monstrosity.clone()) else {
        return;
    };
    let ir = std::sync::Arc::make_mut(&mut sa.ir);
    ir.counter_type_text = Some("P1P1".to_string());
    ir.counter_type = Some(crate::card::CounterType::P1P1);
    ir.semantic_numeric_params.insert(
        keys::COUNTER_NUM.to_string(),
        crate::ability::ability_ir::NumericParamIr::Raw(n),
    );
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `CountersPutEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(CountersPutEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let source_controller = sa
        .source
        .map(|id| ctx.game.card(id).controller)
        .unwrap_or_else(|| ctx.game.player_order[0]);
    let placer = if sa.ir.placer_text.as_deref() == Some("TriggeredSource") {
        sa.get_triggering_player(crate::ability::AbilityKey::Source)
    } else {
        sa.ir.placer_text.as_deref().and_then(|defined| {
            crate::ability::ability_utils::resolve_defined_players_with_sa(
                defined,
                sa,
                source_controller,
                ctx.game,
            )
            .first()
            .copied()
        })
    }
    .unwrap_or(sa.activating_player);
    if sa.ir.triggered_counter_map {
        let Some(crate::event::AbilityValue::CounterMap(counter_map)) = sa
            .trigger_objects
            .get(&crate::ability::AbilityKey::CounterMap)
        else {
            return;
        };
        let Some(card) = resolve_card_targets(ctx.game, sa).first().copied() else {
            return;
        };
        let mut table = GameEntityCounterTable::default();
        for (counter_type, amount) in counter_map {
            table.put(
                Some(placer),
                GameEntity::Card(card),
                parse_counter_type(counter_type),
                sa.ir.counter_map_values.unwrap_or(*amount),
            );
        }
        table.replace_counter_effect(
            ctx.game,
            Some(ctx.trigger_handler),
            Some(ctx.agents),
            Some(sa),
            true,
            RunParams::default(),
        );
        return;
    }
    if crate::parsing::raw_has_key(&sa.ability_text, "EachExistingCounter") {
        let count = resolve_numeric_svar(ctx.game, sa, keys::COUNTER_NUM, 1);
        for player in sa.target_chosen.all_target_players() {
            for counter_type in player_counter_kinds(ctx.game, player) {
                ctx.add_player_counter(
                    player,
                    &counter_type,
                    count,
                    sa,
                    RunParams {
                        source_player: Some(placer),
                        ..Default::default()
                    },
                );
            }
        }
        for card_id in resolve_card_targets(ctx.game, sa) {
            let existing: Vec<crate::card::CounterType> =
                ctx.game.card(card_id).counters.keys().cloned().collect();
            for counter_type in existing {
                if crate::card::card_predicates::can_receive_counters(
                    ctx.game,
                    card_id,
                    &counter_type,
                ) {
                    put_counters_on_card(
                        ctx,
                        sa,
                        card_id,
                        &counter_type,
                        count,
                        placer,
                        source_controller,
                    );
                }
            }
        }
        return;
    }

    if let Some(defined) = crate::parsing::raw_get(&sa.ability_text, "EachFromSource") {
        let sources = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
            ctx.game, sa, defined,
        );
        let explicit = crate::parsing::raw_has_key(&sa.ability_text, "CounterNum");
        let count = resolve_numeric_svar(ctx.game, sa, keys::COUNTER_NUM, 1);
        // Java reads the counters off the object `getDefinedCards` returned (an `LKICopy`, or
        // the host for `Self`), which keeps them after the card moves.
        let from_lki = defined.contains("LKICopy");
        for card_id in resolve_card_targets(ctx.game, sa) {
            for source_card in &sources {
                let host_moved = sa.source == Some(*source_card)
                    && sa.source_zone_timestamp.is_some_and(|created_at| {
                        ctx.game.card(*source_card).zone_timestamp != created_at
                    });
                let counters: Vec<(crate::card::CounterType, i32)> = if from_lki || host_moved {
                    crate::lki::resolve_lki_counters(ctx.game, *source_card)
                } else {
                    ctx.game
                        .card(*source_card)
                        .counters
                        .iter()
                        .map(|(ct, n)| (ct.clone(), *n))
                        .collect()
                };
                for (counter_type, on_source) in counters {
                    let amount = if explicit { count } else { on_source };
                    put_counters_on_card(
                        ctx,
                        sa,
                        card_id,
                        &counter_type,
                        amount,
                        placer,
                        source_controller,
                    );
                }
            }
        }
        return;
    }

    let counter_type_str = sa.ir.counter_type_text.as_deref().unwrap_or("P1P1");
    // Mirror Java CountersPutEffect.java:625-636 — when none of the multi-type
    // dispatch params are present, route the type through the player controller's
    // chooseCounterType prompt (Java's chooseTypeFromList → pc.chooseCounterType).
    // pickOne consumes RNG even for a single option, so calling the agent here
    // keeps deterministic-parity entropy aligned with Java for fixed-type cards
    // like Rottenmouth Viper (CounterType$ BLIGHT).
    let counter_type = if matches_choose_from_list_path(sa) {
        let options: Vec<crate::card::CounterType> = counter_type_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(parse_counter_type)
            .collect();
        if options.is_empty() {
            return;
        }
        ctx.agents[placer.index()].snapshot_state(ctx.game, ctx.mana_pools);
        match ctx.agents[placer.index()].choose_counter_type(
            placer,
            &options,
            "Select counter type",
        ) {
            Some(chosen) => chosen,
            None => return,
        }
    } else {
        sa.ir
            .counter_type
            .clone()
            .unwrap_or_else(|| parse_counter_type(counter_type_str))
    };
    let counter_types: Vec<crate::card::CounterType> = match sa.ir.counter_types_text.as_deref() {
        Some(types) => types
            .split(',')
            .map(str::trim)
            .filter(|type_name| !type_name.is_empty())
            .map(parse_counter_type)
            .collect(),
        None => vec![counter_type],
    };
    if counter_types.is_empty() {
        return;
    }
    if sa.ir.optional {
        let activator = sa.activating_player;
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[activator.index()].confirm_action(
            activator,
            None,
            "Do you want to put the counter?",
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }
    // Support SVar references for CounterNum (e.g. Count$Kicked.4.0 for kicker cards)
    let mut count = resolve_numeric_svar(ctx.game, sa, keys::COUNTER_NUM, 1);
    // Modular death triggers: override the static Modular N with the
    // actual LKI +1/+1 counter count from the dying creature (CR 702.43b).
    // trigger_remembered_amount is set by the death path's LKI capture.
    if sa.ir.modular && sa.trigger_remembered_amount > 0 {
        count = sa.trigger_remembered_amount;
    }

    if crate::parsing::raw_has_key(&sa.ability_text, "ForColor") {
        let Some(source_id) = sa.source else {
            return;
        };
        let old_colors = ctx.game.card(source_id).chosen_colors.clone();
        for color in ["white", "blue", "black", "red", "green"] {
            ctx.game.card_mut(source_id).chosen_colors = vec![color.to_string()];
            resolve_per_type(ctx, sa, &counter_types, count, placer, source_controller);
        }
        ctx.game.card_mut(source_id).chosen_colors = old_colors;
    } else {
        resolve_per_type(ctx, sa, &counter_types, count, placer, source_controller);
    }
}

fn resolve_per_type(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    counter_types: &[crate::card::CounterType],
    count: i32,
    placer: crate::ids::PlayerId,
    source_controller: crate::ids::PlayerId,
) {
    if let Some(filter) = sa.ir.choices.clone() {
        let chooser = match sa.ir.chooser.as_deref() {
            Some(defined) => {
                match crate::ability::ability_utils::resolve_defined_players_with_sa(
                    defined,
                    sa,
                    source_controller,
                    ctx.game,
                )
                .first()
                {
                    Some(&player) => player,
                    None => return,
                }
            }
            None => sa.activating_player,
        };
        let raw = &sa.ability_text;
        let n = crate::parsing::raw_get(raw, "ChoiceAmount")
            .map_or(1, |value| resolve_numeric_value(ctx.game, sa, value, 1));
        let m = crate::parsing::raw_get(raw, "MinChoiceAmount")
            .map_or(n, |value| resolve_numeric_value(ctx.game, sa, value, n));
        if n <= 0 {
            return;
        }
        let zone = sa.ir.choice_zone.unwrap_or(ZoneType::Battlefield);
        let skip_receive = crate::parsing::raw_has_key(raw, "SkipReceiveCounters");
        let mut valid = Vec::new();
        for &pid in &ctx.game.player_order.clone() {
            for cid in ctx.game.cards_in_zone(zone, pid).to_vec() {
                if matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    sa.ir.choices_selector.as_ref(),
                    &filter,
                ) && (skip_receive
                    || crate::card::card_predicates::can_receive_counters(
                        ctx.game,
                        cid,
                        &counter_types[0],
                    ))
                {
                    valid.push(cid);
                }
            }
        }
        ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let chosen = ctx.agents[chooser.index()].choose_cards_for_effect(
            chooser,
            &valid,
            m.max(0) as usize,
            n as usize,
        );
        let divided = crate::parsing::raw_has_key(raw, "DividedAsYouChoose")
            && sa.target_restrictions.is_none();
        let activator = sa.activating_player;
        let mut counter_remain = count;
        for (divrem, &card_id) in chosen.iter().enumerate() {
            let mut amount = count;
            if divided {
                amount = if divrem + 1 == chosen.len() || counter_remain == 1 {
                    counter_remain
                } else {
                    ctx.agents[activator.index()]
                        .choose_number(
                            activator,
                            sa.source,
                            "How many counters",
                            None,
                            1,
                            counter_remain,
                        )
                        .unwrap_or(1)
                };
            }
            for counter_type in counter_types {
                put_counters_on_card(
                    ctx,
                    sa,
                    card_id,
                    counter_type,
                    amount,
                    placer,
                    source_controller,
                );
            }
            if divided {
                counter_remain -= amount;
            }
        }
        return;
    }

    let target_players = match sa.ir.defined.as_ref() {
        Some(defined) => {
            crate::ability::spell_ability_effect::get_defined_entities(ctx.game, sa, defined).0
        }
        None if sa.target_restrictions.is_some() => sa.target_chosen.all_target_players(),
        None => Vec::new(),
    };
    for target_player in target_players {
        for counter_type in counter_types {
            ctx.add_player_counter(
                target_player,
                counter_type,
                count,
                sa,
                RunParams {
                    source_player: Some(placer),
                    ..Default::default()
                },
            );
        }
    }

    // Resolve target card: mirror Java's getDefinedEntitiesOrTargeted().
    // When the SA uses targeting (ValidTgts$), use the chosen target.
    // Otherwise fall back to the Defined$ parameter (default "Self").
    for card_id in resolve_card_targets(ctx.game, sa) {
        for counter_type in counter_types {
            put_counters_on_card(
                ctx,
                sa,
                card_id,
                counter_type,
                count,
                placer,
                source_controller,
            );
        }
    }
}

fn put_counters_on_card(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    card_id: crate::ids::CardId,
    counter_type: &crate::card::CounterType,
    count: i32,
    placer: crate::ids::PlayerId,
    source_controller: crate::ids::PlayerId,
) {
    let is_adapt = sa.ir.adapt.is_some();
    if is_adapt {
        let current = ctx
            .game
            .card(card_id)
            .counter_count(&crate::card::CounterType::P1P1);
        if current > 0
            && !crate::staticability::static_ability_adapt::any_with_adapt(
                &ctx.game.cards,
                sa,
                ctx.game.card(card_id),
            )
        {
            return;
        }
    }

    let is_monstrosity = sa.ir.monstrosity.is_some();
    if is_monstrosity && ctx.game.card(card_id).monstrous {
        return;
    }

    let is_bloodthirst = sa.ir.bloodthirst;
    if is_bloodthirst && !ctx.game.player_has_bloodthirst(source_controller) {
        return;
    }

    if crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
        &ctx.game.cards,
        ctx.game.card(card_id),
        counter_type,
    ) {
        return;
    }
    if let Some(max) = crate::staticability::static_ability_max_counter::max_counter(
        &ctx.game.cards,
        ctx.game.card(card_id),
        counter_type,
    ) {
        let current = ctx.game.card(card_id).counter_count(counter_type);
        if current >= max {
            return;
        }
    }
    let count = if sa.ir.etb {
        ctx.game
            .card_mut(card_id)
            .add_etb_counter(Some(placer), counter_type.clone(), count);
        count
    } else {
        ctx.add_counter(
            card_id,
            counter_type,
            count,
            sa,
            RunParams {
                source_player: Some(placer),
                ..Default::default()
            },
        )
    };

    if crate::parsing::raw_has_key(&sa.ability_text, "RememberCards") {
        if let Some(host) = sa.source {
            ctx.game.card_mut(host).add_remembered_card(card_id);
        }
    }

    if sa.ir.renown && count > 0 {
        ctx.game.card_mut(card_id).set_renowned(true);
    }

    if is_adapt {
        ctx.trigger_handler.run_trigger(
            TriggerType::Adapt,
            RunParams {
                card: Some(card_id),
                ..Default::default()
            },
            false,
        );
    }

    if is_monstrosity {
        ctx.game.card_mut(card_id).set_monstrous(true);
        ctx.trigger_handler.run_trigger(
            TriggerType::BecomeMonstrous,
            RunParams {
                card: Some(card_id),
                counter_amount: Some(count),
                ..Default::default()
            },
            false,
        );
    }
}

fn resolve_card_targets(
    game: &crate::game::GameState,
    sa: &crate::spellability::SpellAbility,
) -> Vec<crate::ids::CardId> {
    let cards: Vec<crate::ids::CardId> =
        if sa.target_restrictions.is_some() && sa.ir.defined.is_none() {
            sa.target_chosen.all_target_cards()
        } else {
            match sa.defined_ref() {
                Some(
                    DefinedRef::TriggeredTarget
                    | DefinedRef::TriggeredTargetLkiCopy
                    | DefinedRef::Targeted,
                ) => sa.target_chosen.all_target_cards(),
                None | Some(DefinedRef::SelfCard) => sa.source.into_iter().collect(),
                Some(_) => {
                    crate::ability::spell_ability_effect::get_defined_cards_or_targeted(game, sa)
                }
            }
        };
    cards
        .into_iter()
        .filter(|&card| {
            let self_moved = matches!(sa.defined_ref(), None | Some(DefinedRef::SelfCard))
                && sa.source == Some(card)
                && sa
                    .source_zone_timestamp
                    .is_some_and(|created_at| game.card(card).zone_timestamp != created_at);
            !self_moved
                && (sa.ir.etb
                    || matches!(sa.defined_ref(), Some(DefinedRef::Remembered))
                    || game.card(card).zone == ZoneType::Battlefield)
        })
        .collect()
}

/// The counter kinds a player can already have. Java reads `Player.getCounters()`; this engine
/// keeps poison, energy and rad as separate fields and the rest in `counters`, so the equivalent
/// is the ones currently above zero.
fn player_counter_kinds(
    game: &crate::game::GameState,
    player: crate::ids::PlayerId,
) -> Vec<crate::card::CounterType> {
    let state = &game.players[player.index()];
    let mut kinds = Vec::new();
    for (amount, name) in [
        (state.poison_counters, "POISON"),
        (state.energy_counters, "ENERGY"),
        (state.radiation_counters, "RAD"),
    ] {
        if amount > 0 {
            kinds.push(parse_counter_type(name));
        }
    }
    kinds.extend(
        state
            .counters
            .iter()
            .filter(|(_, &amount)| amount > 0)
            .map(|(counter_type, _)| counter_type.clone()),
    );
    kinds
}

/// True when CountersPutEffect.java:625-636 would route the CounterType
/// through `chooseTypeFromList` (i.e. `pc.chooseCounterType`). Any of these
/// params steers Java into a different dispatch branch above line 624 or
/// resolves the type without prompting (UniqueType / CounterTypePerDefined
/// also call chooseTypeFromList but inside resolvePerType, not here).
fn matches_choose_from_list_path(sa: &SpellAbility) -> bool {
    #[allow(dead_code)]
    const SKIP_PARAMS: &[&str] = &[
        "EachExistingCounter",
        "EachFromSource",
        "UniqueType",
        "CounterTypePerDefined",
        "CounterTypes",
        "ChooseDifferent",
        "PutOnEachOther",
        "PutOnDefined",
        "TriggeredCounterMap",
        "SharedKeywords",
    ];
    sa.ir.simple_counter_type_choice_path
}

#[cfg(test)]
mod tests {
    use crate::ability::spell_ability_effect::SpellAbilityEffect;
    use crate::HashMap;

    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

    use crate::ability::effects::EffectContext;
    use crate::agent::PassAgent;
    use crate::card::{Card, CounterType};
    use crate::game::GameState;
    use crate::ids::{CardId, PlayerId};
    use crate::mana::ManaPool;
    use crate::spellability::SpellAbility;
    use crate::trigger::handler::TriggerHandler;

    fn make_creature(game: &mut GameState, owner: PlayerId, name: &str) -> CardId {
        let card = Card::new(
            CardId(0),
            name.to_string(),
            owner,
            CardTypeLine::parse("Creature - Golem"),
            ManaCost::parse("5"),
            ColorSet::COLORLESS,
            Some(3),
            Some(3),
            vec![],
            vec![],
        );
        game.create_card(card)
    }

    fn make_ctx<'a>(
        game: &'a mut GameState,
        agents: &'a mut Vec<Box<dyn crate::agent::PlayerAgent>>,
        trigger_handler: &'a mut TriggerHandler,
        mana_pools: &'a mut Vec<ManaPool>,
        token_templates: &'a HashMap<String, Card>,
        token_art_variants: &'a HashMap<(String, String), usize>,
        token_fallback: &'a HashMap<String, String>,
        edition_dates: &'a HashMap<String, String>,
        rng: &'a mut dyn crate::game_rng::GameRng,
    ) -> EffectContext<'a> {
        EffectContext {
            game,
            combat: None,
            agents,
            trigger_handler,
            token_templates,
            token_art_variants,
            token_fallback,
            edition_dates,
            mana_pools,
            parent_target_card: None,
            rng,
        }
    }

    #[test]
    fn monstrosity_only_applies_once() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let clay_golem = make_creature(&mut game, p0, "Clay Golem");
        game.move_card(clay_golem, ZoneType::Battlefield, p0);

        let sa = SpellAbility::new_simple(
            Some(clay_golem),
            p0,
            "AB$ PutCounter | Defined$ Self | Monstrosity$ True | CounterNum$ 4 | CounterType$ P1P1",
        );

        let mut trigger_handler = TriggerHandler::new();
        let mut agents: Vec<Box<dyn crate::agent::PlayerAgent>> =
            vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut mana_pools = vec![ManaPool::default(), ManaPool::default()];
        let token_templates = HashMap::default();
        let templates_variants: HashMap<(String, String), usize> = HashMap::default();
        let token_fallback: HashMap<String, String> = HashMap::default();
        let edition_dates: HashMap<String, String> = HashMap::default();
        let mut rng_adapter = crate::game_rng::ThreadRngAdapter::default();
        let mut ctx = make_ctx(
            &mut game,
            &mut agents,
            &mut trigger_handler,
            &mut mana_pools,
            &token_templates,
            &templates_variants,
            &token_fallback,
            &edition_dates,
            &mut rng_adapter,
        );

        super::CountersPutEffect::resolve(&mut ctx, &sa);
        assert_eq!(
            ctx.game.card(clay_golem).counter_count(&CounterType::P1P1),
            4
        );
        assert!(ctx.game.card(clay_golem).monstrous);

        super::CountersPutEffect::resolve(&mut ctx, &sa);
        assert_eq!(
            ctx.game.card(clay_golem).counter_count(&CounterType::P1P1),
            4
        );
        assert!(ctx.game.card(clay_golem).monstrous);
    }

    #[test]
    fn monstrous_resets_after_leaving_battlefield() {
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let p0 = PlayerId(0);
        let clay_golem = make_creature(&mut game, p0, "Clay Golem");
        game.move_card(clay_golem, ZoneType::Battlefield, p0);
        game.card_mut(clay_golem).set_monstrous(true);

        game.move_card(clay_golem, ZoneType::Hand, p0);

        assert!(!game.card(clay_golem).monstrous);
    }
}
