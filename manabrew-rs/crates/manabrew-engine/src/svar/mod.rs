use crate::ability::ability_ir::{DefinedRef, NumericParamIr};
use crate::card::card_damage_history::TrackedEntity;
use crate::card::filter_constants as fc;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::parsing::compare::compare_expr;
use crate::spellability::SpellAbility;
use forge_card_script::{
    parse_script_svar_numeric_expression, ScriptSVarNumericExpression, ScriptSVarObjectRef,
};

fn parse_trigger_int_values(sa: &SpellAbility, key: &str) -> Vec<i32> {
    crate::ability::ability_key::from_string(key)
        .and_then(|ability_key| sa.get_triggering_value(ability_key))
        .map(|raw| {
            raw.to_trigger_text()
                .split(',')
                .filter_map(|part| part.trim().parse::<i32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn paid_sacrificed_card(sa: &SpellAbility) -> Option<CardId> {
    sa.paid_hash
        .get(crate::cost::cost_sacrifice::HASH_CARDS)
        .or_else(|| sa.paid_hash.get(crate::cost::cost_sacrifice::HASH_LKI))
        .and_then(|ids| ids.first())
        .and_then(|raw| raw.parse::<u32>().ok())
        .map(CardId)
}

fn sacrificed_card_value(game: &GameState, sa: &SpellAbility, svar_expr: &str) -> i32 {
    let Some(sac_id) = paid_sacrificed_card(sa).or(game.last_sacrificed_card) else {
        return 0;
    };
    let sac_card = game.card(sac_id);
    if svar_expr.ends_with("Power") {
        sac_card
            .lki_power
            .unwrap_or(sac_card.base_power.unwrap_or(0))
    } else if svar_expr.ends_with("Toughness") {
        sac_card
            .lki_toughness
            .unwrap_or(sac_card.base_toughness.unwrap_or(0))
    } else {
        sac_card.mana_cost.cmc()
    }
}

fn sacrificed_card_property_value(game: &GameState, sa: &SpellAbility, property: &str) -> i32 {
    match property {
        "CardPower" | "CardToughness" | "CardManaCost" => {
            sacrificed_card_value(game, sa, &format!("Sacrificed${property}"))
        }
        _ => 0,
    }
}

fn apply_simple_operator_chain(num: i32, operators: &str) -> i32 {
    let mut value = num;
    for op in operators.split('/') {
        let op = op.trim();
        if let Some(arg) = op.strip_prefix("Plus.") {
            value += arg.parse::<i32>().unwrap_or(0);
        } else if let Some(arg) = op.strip_prefix("Minus.") {
            value -= arg.parse::<i32>().unwrap_or(0);
        } else if let Some(arg) = op.strip_prefix("Times.") {
            value *= arg.parse::<i32>().unwrap_or(1);
        } else if let Some(arg) = op.strip_prefix("HalfUp") {
            let _ = arg;
            value = (value + 1) / 2;
        } else if let Some(arg) = op.strip_prefix("HalfDown") {
            let _ = arg;
            value = ((value as f64) / 2.0).floor() as i32;
        }
    }
    value
}

fn do_x_math(
    num: i32,
    operators: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    if operators.is_empty() {
        return num;
    }
    let parts: Vec<&str> = operators.split('.').collect();
    let op = parts.first().copied().unwrap_or("");
    let secondary = parts.get(1).copied().map_or(0, |rhs| {
        rhs.parse::<i32>()
            .unwrap_or_else(|_| resolve_svar_expression(rhs, game, source_id, controller, sa))
    });

    if op.contains("Plus") {
        num + secondary
    } else if op.contains("NMinus") {
        secondary - num
    } else if op.contains("Minus") {
        num - secondary
    } else if op.contains("Twice") {
        num * 2
    } else if op.contains("Thrice") {
        num * 3
    } else if op.contains("HalfUp") {
        ((num as f64) / 2.0).ceil() as i32
    } else if op.contains("HalfDown") {
        ((num as f64) / 2.0).floor() as i32
    } else if op.contains("ThirdUp") {
        ((num as f64) / 3.0).ceil() as i32
    } else if op.contains("ThirdDown") {
        ((num as f64) / 3.0).floor() as i32
    } else if op.contains("Negative") {
        -num
    } else if op.contains("Times") {
        num * secondary
    } else if op.contains("Pow") {
        (num as f64).powf(secondary as f64) as i32
    } else if op.contains("DivideEvenlyUp") {
        if secondary == 0 {
            0
        } else {
            num / secondary + i32::from(num % secondary != 0)
        }
    } else if op.contains("DivideEvenlyDown") {
        if secondary == 0 {
            0
        } else {
            num / secondary
        }
    } else if op.contains("Mod") {
        num % secondary
    } else if op.contains("Abs") {
        num.abs()
    } else if op.contains("LimitMax") {
        num.min(secondary)
    } else if op.contains("LimitMin") {
        num.max(secondary)
    } else {
        num
    }
}

fn spell_ability_x_property(spell_ability: &SpellAbility, expr: &str, game: &GameState) -> i32 {
    let Some(source_id) = spell_ability.source else {
        return 0;
    };
    let source = game.card(source_id);
    let left_battlefield = source.zone != forge_foundation::ZoneType::Battlefield;
    let parts: Vec<&str> = expr.split('/').collect();
    let value = parts.first().copied().unwrap_or("");
    let operators = parts.get(1).copied().unwrap_or("");

    let base = match value {
        "CardPower" => left_battlefield
            .then_some(source.lki_power)
            .flatten()
            .unwrap_or_else(|| source.power()),
        "CardToughness" => left_battlefield
            .then_some(source.lki_toughness)
            .flatten()
            .unwrap_or_else(|| source.toughness()),
        "CardNumColors" => source.color.count_colors() as i32,
        _ if value.contains("Converge") => source.sunburst_count(),
        _ if value.starts_with("CardCounters.") => {
            let counter_name = value.strip_prefix("CardCounters.").unwrap_or("");
            let lki_counters = left_battlefield
                .then_some(source.lki_counters.as_ref())
                .flatten();
            if counter_name.eq_ignore_ascii_case("ALL") {
                lki_counters
                    .map(|counters| counters.values().sum())
                    .unwrap_or_else(|| source.num_all_counters())
            } else {
                let counter_type = crate::ability::ability_utils::parse_counter_type(counter_name);
                lki_counters
                    .map(|counters| counters.get(&counter_type).copied().unwrap_or(0))
                    .unwrap_or_else(|| source.counter_count(&counter_type))
            }
        }
        _ if value.starts_with("CardManaCost") => {
            let mut cmc = source.mana_value();
            if value.contains("LKI") && source.zone != forge_foundation::ZoneType::Stack {
                cmc += spell_ability.x_mana_cost_paid as i32 * source.mana_cost.count_x() as i32;
            }
            cmc
        }
        _ => 0,
    };

    do_x_math(
        base,
        operators,
        game,
        source_id,
        spell_ability.activating_player,
        spell_ability,
    )
}

fn card_x_property(
    card_id: CardId,
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    let card = game.card(card_id);
    let parts: Vec<&str> = expr.split('/').collect();
    let value = parts.first().copied().unwrap_or("");
    let operators = parts.get(1).copied().unwrap_or("");

    let in_play = card.zone == forge_foundation::ZoneType::Battlefield;
    let net_power = if in_play {
        card.power()
    } else {
        card.lki_power.unwrap_or_else(|| card.power())
    };
    let net_toughness = if in_play {
        card.toughness()
    } else {
        card.lki_toughness.unwrap_or_else(|| card.toughness())
    };
    let base = match value {
        "CardPower" => net_power,
        "CastTotalManaSpent" => card.paying_mana_to_cast.len() as i32,
        "CardNumColors" => card.color.count_colors() as i32,
        "CardBasePower" => card.base_power.unwrap_or(0),
        "CardToughness" => net_toughness,
        "CardBaseToughness" => card.base_toughness.unwrap_or(0),
        "CardSumPT" => net_power + net_toughness,
        _ if value.starts_with("CardManaCost") || value == "ManaCost" => {
            let mut cmc = card.mana_value();
            if value.contains("LKI") && card.zone != forge_foundation::ZoneType::Stack {
                cmc += sa.x_mana_cost_paid as i32 * card.mana_cost.count_x() as i32;
            }
            cmc
        }
        "Amount" | "Count" => 1,
        _ if value.contains("Converge") => card.sunburst_count(),
        _ if value.starts_with("CardCounters.") => {
            let counter_name = value.strip_prefix("CardCounters.").unwrap_or("");
            if counter_name.eq_ignore_ascii_case("ALL") {
                card.num_all_counters()
            } else {
                card.counter_count(&crate::ability::ability_utils::parse_counter_type(
                    counter_name,
                ))
            }
        }
        _ => 0,
    };

    do_x_math(base, operators, game, source_id, controller, sa)
}

fn resolve_spell_ability_expr(expr: &str, game: &GameState, sa: &SpellAbility) -> Option<i32> {
    let (defined, property) = expr.split_once('$')?;
    resolve_spell_ability_property(defined, property, game, sa)
}

fn resolve_spell_ability_property(
    defined: &str,
    property: &str,
    game: &GameState,
    sa: &SpellAbility,
) -> Option<i32> {
    let spells = crate::ability::ability_utils::get_defined_spell_abilities(defined, sa, game);
    if spells.is_empty() {
        return None;
    }
    Some(
        spells
            .iter()
            .map(|spell| spell_ability_x_property(spell, property, game))
            .sum(),
    )
}

fn resolve_card_list_expr(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> Option<i32> {
    let (defined, property) = expr.split_once('$')?;
    resolve_card_list_property(defined, property, game, source_id, controller, sa)
}

fn resolve_card_list_property(
    defined: &str,
    property: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> Option<i32> {
    let cards = resolve_defined_cards_for_svar(defined, game, source_id, sa);
    if cards.is_empty() && defined != "AllTargeted" {
        return None;
    }
    if let Some(rest) = property.strip_prefix("Valid ") {
        let (valid, operators) = rest.split_once('/').unwrap_or((rest, ""));
        let num = cards
            .into_iter()
            .filter(|&cid| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    game,
                    sa,
                    game.card(cid),
                    None,
                    valid,
                )
            })
            .count() as i32;
        return Some(do_x_math(num, operators, game, source_id, controller, sa));
    }
    let (fold, property) = list_property_fold(property);
    Some(fold(
        cards
            .into_iter()
            .map(|cid| card_x_property(cid, property, game, source_id, controller, sa))
            .collect(),
    ))
}

fn list_property_fold(property: &str) -> (fn(Vec<i32>) -> i32, &str) {
    if let Some(rest) = property.strip_prefix("Least") {
        (|values| values.into_iter().min().unwrap_or(0), rest)
    } else if let Some(rest) = property.strip_prefix("Greatest") {
        (|values| values.into_iter().max().unwrap_or(0), rest)
    } else if let Some(rest) = property.strip_prefix("Different") {
        (
            |mut values| {
                values.sort_unstable();
                values.dedup();
                values.len() as i32
            },
            rest,
        )
    } else {
        (|values| values.into_iter().sum(), property)
    }
}

fn resolve_defined_cards_for_svar(
    defined: &str,
    game: &GameState,
    source_id: CardId,
    sa: &SpellAbility,
) -> Vec<CardId> {
    if defined == "AllTargeted" {
        let mut all = Vec::new();
        let mut current = Some(sa);
        while let Some(node) = current {
            if node.uses_targeting() {
                all.extend(node.target_chosen.all_target_cards());
            }
            current = node.sub_ability.as_deref();
        }
        return all;
    }
    // Java `AbilityUtils.calculateAmount`: `TriggerObjects<Key>` is the whole triggering-object
    // list under that key, where `Triggered<Key>` is the single object.
    if let Some(key) = defined.strip_prefix("TriggerObjects") {
        return crate::ability::ability_key::from_string(key)
            .map(|key| sa.get_triggering_cards(key))
            .unwrap_or_default();
    }

    let defined_ref = DefinedRef::parse(defined);
    match defined_ref {
        DefinedRef::Targeted | DefinedRef::TargetedCard | DefinedRef::ThisTargetedCard => {
            let cards = sa.target_chosen.all_target_cards();
            if !cards.is_empty() {
                return cards;
            }
            // A targeted spell is a stack entry here, where Java holds the card itself.
            sa.target_chosen
                .target_stack_entry
                .and_then(|id| game.stack.find_by_id(id))
                .and_then(|entry| entry.spell_ability.source)
                .into_iter()
                .collect()
        }
        DefinedRef::ParentTargeted => sa.parent_targeting_card.into_iter().collect(),
        DefinedRef::TriggeredCard | DefinedRef::TriggeredCardLkiCopy => {
            let cards = sa.get_triggering_cards(crate::ability::AbilityKey::Card);
            if cards.is_empty() {
                sa.trigger_source.into_iter().collect()
            } else {
                cards
            }
        }
        DefinedRef::ReplacedCard => {
            let cards = sa.get_triggering_cards(crate::ability::AbilityKey::ReplacedCard);
            if cards.is_empty() {
                sa.get_triggering_cards(crate::ability::AbilityKey::Card)
            } else {
                cards
            }
        }
        DefinedRef::TriggeredNewCard | DefinedRef::TriggeredNewCardLkiCopy => {
            let cards = sa.get_triggering_cards(crate::ability::AbilityKey::NewCard);
            if cards.is_empty() {
                sa.trigger_source.into_iter().collect()
            } else {
                cards
            }
        }
        DefinedRef::TriggeredAttacker => {
            sa.get_triggering_cards(crate::ability::AbilityKey::Attacker)
        }
        DefinedRef::TriggeredAttackers => {
            sa.get_triggering_cards(crate::ability::AbilityKey::Attackers)
        }
        DefinedRef::TriggeredBlocker => {
            sa.get_triggering_cards(crate::ability::AbilityKey::Blocker)
        }
        DefinedRef::TriggeredTarget
        | DefinedRef::TriggeredTargetLkiCopy
        | DefinedRef::TriggeredTargets => {
            let cards = sa.get_triggering_cards(crate::ability::AbilityKey::TargetCard);
            if cards.is_empty() {
                sa.get_triggering_cards(crate::ability::AbilityKey::Target)
            } else {
                cards
            }
        }
        DefinedRef::Explorer => sa.get_triggering_cards(crate::ability::AbilityKey::Explorer),
        DefinedRef::Explored => sa.get_triggering_cards(crate::ability::AbilityKey::Explored),
        DefinedRef::Discarded => sa.discarded_cost_cards.clone(),
        DefinedRef::Sacrificed => paid_sacrificed_card(sa)
            .or(game.last_sacrificed_card)
            .into_iter()
            .collect(),
        DefinedRef::Remembered => game.card(source_id).remembered_cards.clone(),
        DefinedRef::RememberedLki => {
            let cards = sa
                .trigger_objects
                .get(&crate::ability::AbilityKey::RememberedLKI)
                .map(cards_from_ability_value)
                .unwrap_or_default();
            if cards.is_empty() {
                game.card(source_id).remembered_cards.clone()
            } else {
                cards
            }
        }
        DefinedRef::DelayTriggerRememberedLki => sa
            .trigger_objects
            .get(&crate::ability::AbilityKey::RememberedLKI)
            .map(cards_from_ability_value)
            .unwrap_or_default(),
        DefinedRef::DelayTriggerRemembered | DefinedRef::TriggerRemembered => sa
            .trigger_remembered
            .iter()
            .flat_map(cards_from_ability_value)
            .collect(),
        DefinedRef::Imprinted => game.card(source_id).imprinted_cards.clone(),
        _ => crate::ability::ability_utils::get_defined_cards(
            game,
            Some(source_id),
            defined_ref.as_legacy_str(),
            Some(sa.activating_player),
        ),
    }
}

fn cards_from_ability_value(value: &crate::event::AbilityValue) -> Vec<CardId> {
    match value {
        crate::event::AbilityValue::Card(cid) => vec![*cid],
        crate::event::AbilityValue::Cards(cards) => cards.clone(),
        _ => Vec::new(),
    }
}

fn resolve_lowered_svar_expression(
    expression: &ScriptSVarNumericExpression<'_>,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> Option<i32> {
    match expression {
        ScriptSVarNumericExpression::Number(value) => {
            let mut parts = value.split('/');
            let number = parts.next().unwrap_or("");
            let operators = parts.next().unwrap_or("");
            Some(do_x_math(
                number.trim().parse::<i32>().unwrap_or(0),
                operators,
                game,
                source_id,
                controller,
                sa,
            ))
        }
        ScriptSVarNumericExpression::Count(raw) => Some(resolve_count_svar_for_sa(
            raw, game, source_id, controller, sa,
        )),
        ScriptSVarNumericExpression::PlayerCount(raw) => Some(resolve_player_count_svar(
            raw, game, source_id, controller, sa,
        )),
        ScriptSVarNumericExpression::TriggerCount(raw) => Some(resolve_trigger_count_svar(
            raw, game, source_id, controller, sa,
        )),
        ScriptSVarNumericExpression::SVarReference { name, operators } => {
            let raw = game.card(source_id).get_s_var(name)?;
            let value = resolve_svar_expression(raw, game, source_id, controller, sa);
            Some(do_x_math(value, operators, game, source_id, controller, sa))
        }
        ScriptSVarNumericExpression::Remembered { property } => {
            let (property, operators) = property.split_once('/').unwrap_or((property, ""));
            let value = crate::ability::ability_utils::handle_paid(
                game,
                &game.card(source_id).remembered_cards,
                property,
                source_id,
            );
            Some(do_x_math(value, operators, game, source_id, controller, sa))
        }
        ScriptSVarNumericExpression::RememberedSize { operators } => Some(do_x_math(
            game.card(source_id).remembered_cards.len() as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        )),
        ScriptSVarNumericExpression::DiscardedValid { filter, times } => Some(
            resolve_discarded_valid_svar(game, source_id, filter, *times),
        ),
        ScriptSVarNumericExpression::ObjectProperty { object, property } => match object {
            ScriptSVarObjectRef::Sacrificed => {
                let (base, operators) = property.split_once('/').unwrap_or((property, ""));
                Some(do_x_math(
                    sacrificed_card_property_value(game, sa, base),
                    operators,
                    game,
                    source_id,
                    controller,
                    sa,
                ))
            }
            ScriptSVarObjectRef::TriggeredCard => {
                let (base, operators) = property.split_once('/').unwrap_or((property, ""));
                crate::lki::resolve_triggered_card_lki_property(game, sa, base)
                    .map(|value| do_x_math(value, operators, game, source_id, controller, sa))
                    .or_else(|| {
                        resolve_card_list_property(
                            "TriggeredCard",
                            property,
                            game,
                            source_id,
                            controller,
                            sa,
                        )
                    })
            }
            ScriptSVarObjectRef::CardList(defined) => {
                resolve_card_list_property(defined, property, game, source_id, controller, sa)
            }
            ScriptSVarObjectRef::PlayerList(defined) => {
                resolve_direct_player_property(defined, property, game, source_id, controller, sa)
            }
            ScriptSVarObjectRef::SpellAbility(defined) => {
                resolve_spell_ability_property(defined, property, game, sa)
            }
            ScriptSVarObjectRef::PaidHash(key) => {
                resolve_paid_hash_property(key, property, game, source_id, sa)
            }
            ScriptSVarObjectRef::ReplaceCount => None,
            ScriptSVarObjectRef::RuntimeValue(_) => None,
        },
        ScriptSVarNumericExpression::Spawner(inner) => match sa.trigger_spawning_ability.as_deref()
        {
            Some(spawner) => {
                resolve_lowered_svar_expression(inner, game, source_id, controller, spawner)
            }
            None => Some(0),
        },
    }
}

fn resolve_discarded_valid_svar(
    game: &GameState,
    source_id: CardId,
    filter: &str,
    times: i32,
) -> i32 {
    let remembered = &game.card(source_id).remembered_cards;
    if remembered.is_empty() {
        return 0;
    }
    for &rem_id in remembered {
        let rem_card = game.card(rem_id);
        let matches = !filter.contains("nonLand") || !rem_card.is_land();
        if matches {
            return times;
        }
    }
    0
}

fn resolve_trigger_count_svar(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    let (prefix, rest) = expr.split_once('$').unwrap_or((expr, ""));
    let mut parts = rest.split('/');
    let key = parts.next().unwrap_or("");
    let operators = parts.next().unwrap_or("");
    let values = parse_trigger_int_values(sa, key.trim());
    let count = if prefix.ends_with("Max") {
        values.into_iter().max().unwrap_or(0)
    } else {
        values.into_iter().sum()
    };
    do_x_math(count, operators, game, source_id, controller, sa)
}

const MAX_SVAR_RESOLUTION_DEPTH: usize = 50;

thread_local! {
    static SVAR_RESOLUTION_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct SvarResolutionDepthGuard {
    previous: usize,
}

impl Drop for SvarResolutionDepthGuard {
    fn drop(&mut self) {
        SVAR_RESOLUTION_DEPTH.with(|d| d.set(self.previous));
    }
}

pub(crate) fn resolve_svar_expression(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    let depth = SVAR_RESOLUTION_DEPTH.with(|d| d.get());
    if depth >= MAX_SVAR_RESOLUTION_DEPTH {
        eprintln!("SVar resolution exceeded depth limit, returning 0 for: {expr}");
        return 0;
    }
    SVAR_RESOLUTION_DEPTH.with(|d| d.set(depth + 1));
    let _depth_guard = SvarResolutionDepthGuard { previous: depth };
    resolve_svar_expression_inner(expr, game, source_id, controller, sa)
}

fn resolve_svar_expression_inner(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    let expr = expr.trim();
    if let Ok(n) = expr.parse::<i32>() {
        return n;
    }
    if let Some(expression) = parse_script_svar_numeric_expression(expr) {
        if let Some(value) =
            resolve_lowered_svar_expression(&expression, game, source_id, controller, sa)
        {
            return value;
        }
    }
    if expr.starts_with("TriggerCount$") || expr.starts_with("TriggerCountMax$") {
        return resolve_trigger_count_svar(expr, game, source_id, controller, sa);
    }
    if expr.starts_with("Count$") {
        return resolve_count_svar_for_sa(expr, game, source_id, controller, sa);
    }
    if expr.starts_with("PlayerCount") {
        return resolve_player_count_svar(expr, game, source_id, controller, sa);
    }
    if let Some(property) = expr.strip_prefix("Remembered$") {
        let (property, operators) = property.split_once('/').unwrap_or((property, ""));
        let value = crate::ability::ability_utils::handle_paid(
            game,
            &game.card(source_id).remembered_cards,
            property,
            source_id,
        );
        return do_x_math(value, operators, game, source_id, controller, sa);
    }
    if let Some(rest) = expr.strip_prefix("RememberedSize") {
        return do_x_math(
            game.card(source_id).remembered_cards.len() as i32,
            rest.strip_prefix('/').unwrap_or(""),
            game,
            source_id,
            controller,
            sa,
        );
    }
    if let Some(value) = resolve_paid_hash_expr(expr, game, source_id, sa) {
        return value;
    }
    if let Some(value) = resolve_spell_ability_expr(expr, game, sa) {
        return value;
    }
    if let Some(value) = resolve_card_list_expr(expr, game, source_id, controller, sa) {
        return value;
    }
    if let Some(value) = crate::lki::resolve_triggered_card_lki_svar(game, sa, expr) {
        return value;
    }
    if let Some(value) = resolve_direct_player_expr(expr, game, source_id, controller, sa) {
        return value;
    }
    if let Some(svar_expr) = game.card(source_id).get_s_var(expr) {
        return resolve_svar_expression(svar_expr, game, source_id, controller, sa);
    }
    0
}

fn player_x_property(
    player: PlayerId,
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    let parts: Vec<&str> = expr.split('/').collect();
    let value = parts.first().copied().unwrap_or("");
    let operators = parts.get(1).copied().unwrap_or("");

    let base = match value {
        _ if value.starts_with("SacrificedThisTurn") => {
            let sacrificed = &game.player(player).sacrificed_this_turn;
            match value.split_once(' ') {
                Some((_, restrictions)) => {
                    let selector = crate::parsing::cached_compiled_selector(restrictions);
                    let context =
                        crate::card::valid_filter::MatchContext::from_source(game.card(source_id))
                            .with_game(game)
                            .with_source_controller(player);
                    sacrificed
                        .iter()
                        .filter(|card| {
                            crate::card::valid_filter::matches_valid_card_selector_with_context(
                                &selector, card, context,
                            )
                        })
                        .count() as i32
                }
                None => sacrificed.len() as i32,
            }
        }
        _ if value.starts_with("Valid") => {
            let (zones, restrictions) = if let Some(rest) = value.strip_prefix("Valid ") {
                (vec![forge_foundation::ZoneType::Battlefield], rest)
            } else {
                let mut parts = value.splitn(2, ' ');
                let zone_part = parts
                    .next()
                    .unwrap_or("")
                    .strip_prefix("Valid")
                    .unwrap_or("");
                let restrictions = parts.next().unwrap_or("");
                let zones: Vec<_> = if zone_part.is_empty() {
                    vec![forge_foundation::ZoneType::Battlefield]
                } else {
                    zone_part
                        .split(',')
                        .filter_map(crate::ability::ability_utils::parse_zone_type)
                        .collect()
                };
                (zones, restrictions)
            };
            let selector = crate::parsing::cached_compiled_selector(restrictions);
            let source = game.card(source_id);
            // Mirror Java `AbilityUtils.playerXProperty` (`AbilityUtils.java:
            // 3380, 3389`): pass the iterated `player` as the `YouCtrl`
            // controller so per-opponent counts (e.g. Beza's
            // `PlayerCountOpponents$HighestValid Land.YouCtrl`) actually scope
            // to that opponent's permanents, not the source's.
            let context = crate::card::valid_filter::MatchContext::from_source(source)
                .with_game(game)
                .with_source_controller(player);
            game.cards
                .iter()
                .filter(|card| {
                    zones.contains(&card.zone)
                        && crate::card::valid_filter::matches_valid_card_selector_with_context(
                            &selector, card, context,
                        )
                })
                .count() as i32
        }
        "CardsInHand" => game
            .cards_in_zone(forge_foundation::ZoneType::Hand, player)
            .len() as i32,
        "CardsInLibrary" => game
            .cards_in_zone(forge_foundation::ZoneType::Library, player)
            .len() as i32,
        "CardsInGraveyard" => game
            .cards_in_zone(forge_foundation::ZoneType::Graveyard, player)
            .len() as i32,
        "CardsInPlay" => game
            .cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
            .len() as i32,
        "CreaturesInPlay" => game
            .cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
            .iter()
            .filter(|&&cid| game.card(cid).is_creature())
            .count() as i32,
        "StartingLife" => game.player(player).starting_life,
        "LifeTotal" => game.player(player).life,
        "LifeLostThisTurn" => game.player(player).life_lost_this_turn,
        "LifeLostLastTurn" => game.player(player).life_lost_last_turn,
        "LifeGainedThisTurn" => game.player(player).life_gained_this_turn,
        "LifeGainedByTeamThisTurn" => game.player(player).life_gained_by_team_this_turn,
        "LifeStartedThisTurnWith" => game.player(player).life_started_this_turn_with,
        "Speed" => game.player(player).speed,
        "TopOfLibraryCMC" => game
            .cards_in_zone(forge_foundation::ZoneType::Library, player)
            .last()
            .map(|&cid| game.card(cid).mana_value())
            .unwrap_or(0),
        "LandsPlayed" => game.player(player).lands_played_this_turn,
        "SpellsCastThisTurn" => game.player(player).spells_cast_this_turn,
        "CardsDrawn" => game.player(player).drawn_this_turn,
        "CardsDiscardedThisTurn" => game.player(player).discarded_this_turn,
        "ExploredThisTurn" => game.player(player).explored_this_turn,
        "AttackersDeclared" => game
            .cards
            .iter()
            .filter(|card| {
                card.controller == player && card.attacked_this_turn && card.is_creature()
            })
            .count() as i32,
        "DamageToOppsThisTurn" => game.player(player).opponents_assigned_damage_this_turn,
        "NonCombatDamageDealtThisTurn" => {
            game.player(player).assigned_damage_this_turn
                - game.player(player).assigned_combat_damage_this_turn
        }
        "PoisonCounters" => game.player(player).poison_counters,
        "EnergyCounters" => game.player(player).energy_counters,
        "ManaExpendedThisTurn" => game.player(player).mana_expended_this_turn,
        "RingTemptedYou" => game.player(player).ring_level,
        "OpponentsAttackedThisTurn" => {
            let mut attacked = Vec::new();
            for card in &game.cards {
                if card.controller != player {
                    continue;
                }
                for entity in &card.damage_history.attacked_this_turn {
                    if let TrackedEntity::Player(pid) = entity {
                        if !attacked.contains(pid) {
                            attacked.push(*pid);
                        }
                    }
                }
            }
            attacked.len() as i32
        }
        "OpponentsAttackedThisCombat" => {
            game.player(player).attacked_players_this_combat.len() as i32
        }
        "BeenDealtCombatDamageSinceLastTurn" => {
            i32::from(game.player(player).been_dealt_combat_damage_since_last_turn)
        }
        "AttractionsVisitedThisTurn" => game.player(player).attractions_visited_this_turn,
        _ if value.starts_with("Counters.") => {
            let counter_name = value.strip_prefix("Counters.").unwrap_or("");
            if counter_name.eq_ignore_ascii_case("ALL") {
                game.player(player).poison_counters
                    + game.player(player).energy_counters
                    + game.player(player).radiation_counters
            } else if counter_name.eq_ignore_ascii_case("POISON") {
                game.player(player).poison_counters
            } else if counter_name.eq_ignore_ascii_case("ENERGY") {
                game.player(player).energy_counters
            } else if counter_name.eq_ignore_ascii_case("RADIATION") {
                game.player(player).radiation_counters
            } else {
                0
            }
        }
        _ if value.starts_with("HasProperty") => i32::from(crate::player::player_has_property(
            player,
            value.strip_prefix("HasProperty").unwrap_or(""),
            game,
            source_id,
            controller,
            sa,
        )),
        _ => 0,
    };

    do_x_math(base, operators, game, source_id, controller, sa)
}

pub fn player_condition_matches(
    player: PlayerId,
    property: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> bool {
    let Some(rest) = property.strip_prefix("Condition") else {
        return false;
    };
    let Some((lhs, prop_expr)) = rest.split_once(' ') else {
        return false;
    };
    let (cmp, rhs_expr) = if lhs.is_empty() {
        ("GE", "1")
    } else if lhs.len() >= 2 {
        (&lhs[..2], &lhs[2..])
    } else {
        ("GE", "1")
    };
    let rhs = resolve_svar_expression(rhs_expr, game, source_id, controller, sa);
    compare_expr(
        player_x_property(player, prop_expr, game, source_id, controller, sa),
        &format!("{cmp}{rhs}"),
    )
}

fn resolve_direct_player_expr(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> Option<i32> {
    let (defined, property) = expr.split_once('$')?;
    resolve_direct_player_property(defined, property, game, source_id, controller, sa)
}

fn resolve_direct_player_property(
    defined: &str,
    property: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> Option<i32> {
    let players = crate::ability::ability_utils::resolve_defined_players_with_sa(
        defined, sa, controller, game,
    );
    if players.is_empty() {
        return None;
    }
    Some(
        players
            .into_iter()
            .map(|pid| player_x_property(pid, property, game, source_id, controller, sa))
            .sum(),
    )
}

fn resolve_player_count_svar(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    let Some((group, property_expr)) = expr.split_once('$') else {
        return 0;
    };
    let kind = group.strip_prefix("PlayerCount").unwrap_or(group);
    let mut property_parts = property_expr.splitn(2, '/');
    let property = property_parts.next().unwrap_or("");
    let operators = property_parts.next().unwrap_or("");
    let players: Vec<PlayerId> = if kind.is_empty() || kind == "Players" {
        game.alive_players()
    } else if kind == "Opponents" {
        game.alive_players()
            .into_iter()
            .filter(|&pid| crate::player::player_predicates::is_opponent_of(game, controller, pid))
            .collect()
    } else if kind == "RegisteredOpponents" {
        // Java reads `game.getRegisteredPlayers()` here, not the living ones
        // (`AbilityUtils.java:465`), so a player who has already lost still counts.
        game.player_order
            .iter()
            .copied()
            .filter(|&pid| crate::player::player_predicates::is_opponent_of(game, controller, pid))
            .collect()
    } else if kind == "Remembered" {
        game.card(source_id).remembered_players.clone()
    } else if kind.starts_with("PropertyYou") {
        vec![controller]
    } else if let Some(property) = kind.strip_prefix("Property") {
        game.alive_players()
            .into_iter()
            .filter(|&pid| {
                crate::player::player_has_property(pid, property, game, source_id, controller, sa)
            })
            .collect()
    } else if let Some(defined) = kind.strip_prefix("Defined") {
        crate::ability::ability_utils::resolve_defined_players_with_sa(
            defined, sa, controller, game,
        )
    } else {
        Vec::new()
    };

    if players.is_empty() {
        return 0;
    }

    if property.eq_ignore_ascii_case("Amount") {
        return do_x_math(
            players.len() as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    if let Some(rest) = property.strip_prefix("Highest") {
        return do_x_math(
            players
                .iter()
                .map(|&pid| player_x_property(pid, rest, game, source_id, controller, sa))
                .max()
                .unwrap_or(0),
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    if let Some(rest) = property.strip_prefix("Lowest") {
        return do_x_math(
            players
                .iter()
                .map(|&pid| player_x_property(pid, rest, game, source_id, controller, sa))
                .min()
                .unwrap_or(0),
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    if property.eq_ignore_ascii_case("TiedForHighestLife") {
        let max_life = players
            .iter()
            .map(|&pid| game.player(pid).life)
            .max()
            .unwrap_or(i32::MIN);
        return do_x_math(
            players
                .iter()
                .filter(|&&pid| game.player(pid).life == max_life)
                .count() as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    if property.eq_ignore_ascii_case("TiedForLowestLife") {
        let min_life = players
            .iter()
            .map(|&pid| game.player(pid).life)
            .min()
            .unwrap_or(i32::MAX);
        return do_x_math(
            players
                .iter()
                .filter(|&&pid| game.player(pid).life == min_life)
                .count() as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    if let Some(raw_property) = property.strip_prefix("HasProperty") {
        return do_x_math(
            players
                .into_iter()
                .filter(|&pid| {
                    crate::player::player_has_property(
                        pid,
                        raw_property,
                        game,
                        source_id,
                        controller,
                        sa,
                    )
                })
                .count() as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    if let Some(rest) = property.strip_prefix("Condition") {
        if let Some((lhs, prop_expr)) = rest.split_once(' ') {
            let (cmp, rhs_expr) = if lhs.is_empty() {
                ("GE", "1")
            } else if lhs.len() >= 2 {
                (&lhs[..2], &lhs[2..])
            } else {
                ("GE", "1")
            };
            let rhs = resolve_svar_expression(rhs_expr, game, source_id, controller, sa);
            return do_x_math(
                players
                    .into_iter()
                    .filter(|&pid| {
                        compare_expr(
                            player_x_property(pid, prop_expr, game, source_id, controller, sa),
                            &format!("{cmp}{rhs}"),
                        )
                    })
                    .count() as i32,
                operators,
                game,
                source_id,
                controller,
                sa,
            );
        }
    }

    do_x_math(
        players
            .into_iter()
            .map(|pid| player_x_property(pid, property, game, source_id, controller, sa))
            .sum(),
        operators,
        game,
        source_id,
        controller,
        sa,
    )
}

/// Resolve a numeric parameter from a SpellAbility, expanding SVar references.
///
/// This is the main entry point for effect resolution — call it whenever you
/// need to convert a param value (which might be a literal int, "X", or an
/// SVar reference) into an integer.
///
/// **Examples:**
/// - `"NumDmg" -> "3"` → returns 3
/// - `"NumDmg" -> "X"` → returns `sa.x_mana_cost_paid` or evaluates the "X" SVar
/// - `"NumDmg" -> "AFLifeLost"` → looks up SVar "AFLifeLost" and evaluates it
///
/// **param_name**: The param key on the ability IR (e.g. "NumDmg", "LifeAmount")
/// **default**: The value to return if the param is missing or empty
pub fn resolve_numeric_svar(
    game: &GameState,
    sa: &SpellAbility,
    param_name: &str,
    default: i32,
) -> i32 {
    let Some(value) = sa.ir.semantic_numeric_params.get(param_name) else {
        return default;
    };
    resolve_semantic_numeric_value(game, sa, value, default)
}

fn resolve_semantic_numeric_value(
    game: &GameState,
    sa: &SpellAbility,
    value: &NumericParamIr,
    default: i32,
) -> i32 {
    match value {
        NumericParamIr::Integer(value) => *value,
        NumericParamIr::Amount(amount) => amount.resolve_for_spell_ability(game, sa, default),
        NumericParamIr::SVarReference(names) => match names.as_slice() {
            [name] => resolve_numeric_value(game, sa, name, default),
            [] => default,
            _ => names
                .iter()
                .map(|name| resolve_numeric_value(game, sa, name, default))
                .sum(),
        },
        NumericParamIr::Raw(raw) => resolve_numeric_value(game, sa, raw, default),
    }
}

/// Resolve a raw numeric DSL value using the same semantics as
/// [`resolve_numeric_svar`], without first looking it up in `sa.params`.
pub fn resolve_numeric_value(
    game: &GameState,
    sa: &SpellAbility,
    raw_val: &str,
    default: i32,
) -> i32 {
    let val_str = raw_val.trim();
    if val_str.is_empty() {
        return default;
    }

    // Try direct integer parse first
    if let Ok(n) = val_str.parse::<i32>() {
        return n;
    }
    // Try with leading + sign (e.g. "+3")
    if let Some(stripped) = val_str.strip_prefix('+') {
        if let Ok(n) = stripped.parse::<i32>() {
            return n;
        }
    }

    // Support signed SVar references like "-X" / "+X".
    let (sign, val_str) = if let Some(stripped) = val_str.strip_prefix('-') {
        (-1, stripped.trim())
    } else if let Some(stripped) = val_str.strip_prefix('+') {
        (1, stripped.trim())
    } else {
        (1, val_str)
    };

    if let Some(source_id) = sa.source {
        if let Some(expression) = parse_script_svar_numeric_expression(val_str) {
            if let Some(value) = resolve_lowered_svar_expression(
                &expression,
                game,
                source_id,
                sa.activating_player,
                sa,
            ) {
                return sign * value;
            }
        }
        if let Some(value) =
            resolve_card_list_expr(val_str, game, source_id, sa.activating_player, sa)
        {
            return sign * value;
        }
    }

    // Check if it's the X mana cost value directly
    if val_str == "X" {
        // First check if there's an SVar named "X" on the source card
        if let Some(source_id) = sa.source {
            if let Some(svar_expr) = crate::ability::ability_utils::get_s_var(sa, game, "X") {
                if svar_expr.starts_with("Count$") {
                    return sign
                        * resolve_count_svar_for_sa(
                            svar_expr,
                            game,
                            source_id,
                            sa.activating_player,
                            sa,
                        );
                }
                if svar_expr.starts_with("PlayerCount") {
                    return sign
                        * resolve_player_count_svar(
                            svar_expr,
                            game,
                            source_id,
                            sa.activating_player,
                            sa,
                        );
                }
                if let Some(value) = resolve_paid_hash_expr(svar_expr, game, source_id, sa) {
                    return sign * value;
                }
                if svar_expr.starts_with("TriggerCount$")
                    || svar_expr.starts_with("TriggerCountMax$")
                {
                    return sign
                        * resolve_trigger_count_svar(
                            svar_expr,
                            game,
                            source_id,
                            sa.activating_player,
                            sa,
                        );
                }
                if let Some(expression) = parse_script_svar_numeric_expression(svar_expr) {
                    if let Some(value) = resolve_lowered_svar_expression(
                        &expression,
                        game,
                        source_id,
                        sa.activating_player,
                        sa,
                    ) {
                        return sign * value;
                    }
                }
                if let Some(value) = resolve_spell_ability_expr(svar_expr, game, sa) {
                    return sign * value;
                }
                if let Some(value) =
                    resolve_card_list_expr(svar_expr, game, source_id, sa.activating_player, sa)
                {
                    return sign * value;
                }
                // Must run before resolve_direct_player_expr, which can
                // greedily match some object-property expression prefixes.
                if let Some(value) =
                    crate::lki::resolve_triggered_card_lki_svar(game, sa, svar_expr)
                {
                    return sign * value;
                }
                if let Some(value) =
                    resolve_direct_player_expr(svar_expr, game, source_id, sa.activating_player, sa)
                {
                    return sign * value;
                }
                return sign * evaluate_svar(svar_expr, sa);
            }
        }
        // Otherwise use x_mana_cost_paid directly
        return sign * sa.x_mana_cost_paid as i32;
    }

    // It's an SVar reference — look it up on the source card
    if let Some(source_id) = sa.source {
        if let Some(svar_expr) = crate::ability::ability_utils::get_s_var(sa, game, val_str.trim())
        {
            // Game-aware SVar resolution for patterns that need GameState.
            if svar_expr.starts_with("Count$") {
                return sign
                    * resolve_count_svar_for_sa(
                        svar_expr,
                        game,
                        source_id,
                        sa.activating_player,
                        sa,
                    );
            }
            if svar_expr.starts_with("PlayerCount") {
                return sign
                    * resolve_player_count_svar(
                        svar_expr,
                        game,
                        source_id,
                        sa.activating_player,
                        sa,
                    );
            }
            if let Some(value) = resolve_paid_hash_expr(svar_expr, game, source_id, sa) {
                return sign * value;
            }
            if let Some(expression) = parse_script_svar_numeric_expression(svar_expr) {
                if let Some(value) = resolve_lowered_svar_expression(
                    &expression,
                    game,
                    source_id,
                    sa.activating_player,
                    sa,
                ) {
                    return sign * value;
                }
            }
            if let Some(value) = resolve_spell_ability_expr(svar_expr, game, sa) {
                return sign * value;
            }
            if let Some(value) =
                resolve_card_list_expr(svar_expr, game, source_id, sa.activating_player, sa)
            {
                return sign * value;
            }
            // Must be checked before resolve_direct_player_expr, which
            // would incorrectly match "TriggeredCard" as a player definition.
            if let Some(value) = crate::lki::resolve_triggered_card_lki_svar(game, sa, svar_expr) {
                return sign * value;
            }
            // evaluate_svar handles Number$N, Count$Kicked, TriggerCount, etc.
            // Must run before resolve_direct_player_expr which greedily matches
            // any foo$bar pattern via the resolve_defined_players fallback.
            let eval = evaluate_svar(svar_expr, sa);
            if eval != 0 || svar_expr.starts_with("Number$") || svar_expr.starts_with("Count$") {
                return sign * eval;
            }
            if let Some(value) =
                resolve_direct_player_expr(svar_expr, game, source_id, sa.activating_player, sa)
            {
                return sign * value;
            }
            return sign * eval;
        }
    }

    default
}

fn resolve_paid_hash_expr(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    sa: &SpellAbility,
) -> Option<i32> {
    let (paid_key, property) = expr.split_once('$')?;
    resolve_paid_hash_property(paid_key, property, game, source_id, sa)
}

fn resolve_paid_hash_property(
    paid_key: &str,
    property: &str,
    game: &GameState,
    source_id: CardId,
    sa: &SpellAbility,
) -> Option<i32> {
    let paid_values = sa.paid_hash.get(paid_key)?;
    let paid_cards: Vec<CardId> = paid_values
        .iter()
        .filter_map(|value| {
            let raw = value.strip_prefix("Card#").unwrap_or(value);
            raw.parse::<u32>().ok().map(CardId)
        })
        .filter(|cid| cid.index() < game.cards.len())
        .collect();

    if property.starts_with("TapPowerValue") {
        return Some(
            paid_cards
                .iter()
                .map(|&cid| crate::cost::cost_tap_type::tap_power_value(game, cid, Some(sa)))
                .sum(),
        );
    }

    Some(crate::ability::ability_utils::handle_paid(
        game,
        &paid_cards,
        property,
        source_id,
    ))
}

/// Evaluate a simple SVar expression.
/// Supports `Count$Kicked.A.B` (returns A if kicked, B otherwise)
/// and `Count$KickedCount` (returns the multikicker count).
pub fn evaluate_svar(expr: &str, sa: &SpellAbility) -> i32 {
    // X mana cost — return the value of X paid when casting
    if let Some(rest) = expr
        .strip_prefix("Count$xPaid")
        .or_else(|| expr.strip_prefix("Count$XPaid"))
    {
        let operators = rest.strip_prefix('/').unwrap_or(rest);
        return apply_simple_operator_chain(sa.x_mana_cost_paid as i32, operators);
    }
    // Converge/Sunburst — handled in resolve_numeric_svar (needs GameState)
    if expr == "Count$Converge" || expr == "Count$Sunburst" {
        return 0; // Fallback; game-aware path in resolve_numeric_svar handles this
    }
    if expr == "Count$TriggerRememberAmount" {
        return sa.trigger_remembered_amount;
    }
    if let Some(rest) = expr.strip_prefix("TriggerCount$") {
        let (key, operators) = rest.split_once('/').unwrap_or((rest, ""));
        let values = parse_trigger_int_values(sa, key.trim());
        let count = values.into_iter().sum::<i32>();
        return apply_simple_operator_chain(count, operators);
    }
    if let Some(rest) = expr.strip_prefix("TriggerCountMax$") {
        let (key, operators) = rest.split_once('/').unwrap_or((rest, ""));
        let count = parse_trigger_int_values(sa, key.trim())
            .into_iter()
            .max()
            .unwrap_or(0);
        return apply_simple_operator_chain(count, operators);
    }
    if expr == "TriggerCount$Result" {
        return trigger_result_values(sa).into_iter().sum();
    }
    if expr == "TriggerCountMax$Result" {
        return trigger_result_values(sa).into_iter().max().unwrap_or(0);
    }
    // TriggerCount$Amount — number of objects that matched the trigger event.
    // For per-event triggers (ChangesZoneAll batched as individual fires), this is 1.
    if expr == "TriggerCount$Amount" {
        return sa.trigger_remembered_amount.max(1);
    }
    // Count$KickedCount — return the multikicker count (for Multikicker effects)
    if expr == "Count$KickedCount" {
        return sa.kick_count as i32;
    }
    // Count$Kicked.X.Y — if kicked return X, else return Y
    if let Some(rest) = expr.strip_prefix("Count$Kicked.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let kicked_val = parts[0].parse::<i32>().unwrap_or(0);
            let normal_val = parts[1].parse::<i32>().unwrap_or(0);
            return if sa.kicked { kicked_val } else { normal_val };
        }
    }
    // Number$N — literal numeric SVar (e.g. "Number$2" set by LoseLife for AFLifeLost)
    if let Some(rest) = expr.strip_prefix("Number$") {
        return rest.trim().parse::<i32>().unwrap_or(0);
    }
    // Fallback: try parsing as integer
    expr.parse::<i32>().unwrap_or(0)
}

fn trigger_result_values(sa: &SpellAbility) -> Vec<i32> {
    sa.get_triggering_value(crate::ability::AbilityKey::Result)
        .map(|raw| {
            raw.to_trigger_text()
                .split(',')
                .filter_map(|part| part.trim().parse::<i32>().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Resolve a Count$ SVar expression that requires game state access.
/// Handles patterns like `Count$Valid Forest.YouCtrl`, `Count$Converge`,
/// `Count$CardPower`, etc.
pub fn resolve_count_svar(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
) -> i32 {
    resolve_count_svar_for_sa(
        expr,
        game,
        source_id,
        controller,
        &crate::spellability::SpellAbility::new_empty(Some(source_id), controller),
    )
}

/// Resolve a cost-adjustment `Amount$ <ident>` slot. Tries a direct integer
/// parse first, then looks up `ident` on the host's SVar table and routes
/// through the subset of `Count$…` patterns relevant for cost adjustment.
/// Mirrors Java `AbilityUtils.calculateAmount(host, name, staticAbility)`
/// for the static-ability path. Owned by `svar/` so cost callers don't
/// reimplement SVar walking.
pub fn resolve_cost_amount_svar(
    game: &GameState,
    source: &crate::card::Card,
    name: &str,
    caster: PlayerId,
) -> i32 {
    if let Ok(n) = name.parse::<i32>() {
        return n;
    }
    let Some(expr) = source.get_s_var(name) else {
        return 0;
    };
    evaluate_cost_amount_count_expr(game, source, expr, caster)
}

fn evaluate_cost_amount_count_expr(
    game: &GameState,
    source: &crate::card::Card,
    expr: &str,
    caster: PlayerId,
) -> i32 {
    use crate::card::Card;
    use forge_foundation::ZoneType;
    if expr == "Count$xPaid" || expr == "Count$XPaid" {
        return source
            .svars
            .get("XPaid")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);
    }
    if let Some(counter_name) = expr.strip_prefix("Count$CardCounters.") {
        if counter_name == "ALL" {
            return source.num_all_counters();
        }
        let counter_type = crate::ability::ability_utils::parse_counter_type(counter_name);
        return source.counter_count(&counter_type);
    }
    if let Some(rest) = expr.strip_prefix("Count$ThisTurnCast_") {
        if rest.contains("YouCtrl") || rest.contains("YouOwn") {
            return game.player(source.controller).spells_cast_this_turn;
        }
        return game.player(caster).spells_cast_this_turn;
    }
    if expr == "Count$YourLifeTotal" {
        return game.player(source.controller).life;
    }
    if let Some(rest) = expr.strip_prefix("Count$Valid ") {
        let (filter, aggregator) = rest.split_once('$').unwrap_or((rest, ""));
        let selector = crate::parsing::cached_compiled_selector(filter);
        let matches: Vec<&Card> = game
            .cards
            .iter()
            .filter(|c| c.zone == ZoneType::Battlefield)
            .filter(|c| {
                crate::card::valid_filter::matches_valid_card_selector_in_game(
                    &selector, c, source, game,
                )
            })
            .collect();
        return count_valid_aggregate(game, &matches, aggregator);
    }
    if let Some(n) = expr
        .strip_prefix("Count$")
        .and_then(|s| s.parse::<i32>().ok())
    {
        return n;
    }
    crate::ability::effects::resolve_count_svar(expr, game, source.id, source.controller)
}

/// The part of `AbilityUtils.handlePaid` that a `Count$Valid... $<what>` suffix reaches; the tail
/// is Java's Least/Greatest/Different/sum over `xCount(card, <what>)` for the card properties
/// listed.
fn count_valid_aggregate(
    game: &GameState,
    matches: &[&crate::card::Card],
    aggregator: &str,
) -> i32 {
    match aggregator {
        "" | "Amount" => matches.len() as i32,
        "GreatestCardManaCost" => matches.iter().map(|c| c.mana_cost.cmc()).max().unwrap_or(0),
        "CardTypes" | "CardTypesPermanent" => {
            let ids: Vec<CardId> = matches.iter().map(|c| c.id).collect();
            crate::ability::ability_utils::count_card_types_from_list(
                game,
                &ids,
                aggregator == "CardTypesPermanent",
            )
        }
        "Colors" => matches
            .iter()
            .fold(0u8, |mask, card| mask | card.color.mask())
            .count_ones() as i32,
        "CreatureType" => {
            let mut creature_types: Vec<&str> = Vec::new();
            for card in matches {
                for subtype in &card.type_line.subtypes {
                    if crate::game::TypeRegistry::creature_types()
                        .iter()
                        .any(|ct| ct.eq_ignore_ascii_case(subtype))
                        && !creature_types.contains(&subtype.as_str())
                    {
                        creature_types.push(subtype);
                    }
                }
            }
            creature_types.len() as i32
        }
        "DifferentCardNames" => {
            let mut names: Vec<&str> = Vec::new();
            for card in matches.iter().filter(|card| !card.face_down) {
                if !names.contains(&card.card_name.as_str()) {
                    names.push(card.card_name.as_str());
                }
            }
            names.len() as i32
        }
        other => {
            let (fold, property) = list_property_fold(other);
            let values: Option<Vec<i32>> = matches
                .iter()
                .map(|card| match property {
                    "CardPower" => Some(card.power()),
                    "CardToughness" => Some(card.toughness()),
                    "CardSumPT" => Some(card.power() + card.toughness()),
                    "CardManaCost" => Some(card.mana_value()),
                    _ if property.starts_with("CardCounters.") => {
                        let counter_name = property.strip_prefix("CardCounters.").unwrap_or("");
                        Some(if counter_name.eq_ignore_ascii_case("ALL") {
                            card.num_all_counters()
                        } else {
                            card.counter_count(&crate::ability::ability_utils::parse_counter_type(
                                counter_name,
                            ))
                        })
                    }
                    _ => None,
                })
                .collect();
            match values {
                Some(values) => fold(values),
                None => {
                    crate::census::unhandled("count_valid_aggregate", other);
                    0
                }
            }
        }
    }
}

/// CR 107.3k as `AbilityUtils.xCount` applies it: an enters-the-battlefield trigger reads X from
/// the card, which keeps the X paid for the spell that became it.
fn enters_trigger_x_paid(game: &GameState, sa: &SpellAbility, card_id: CardId) -> Option<i32> {
    let host = sa.trigger_source.unwrap_or(card_id);
    let trigger = game.card(host).triggers.get(sa.trigger_index?)?;
    (trigger.mode.trigger_type() == crate::trigger::TriggerType::ChangesZone
        && trigger
            .ir
            .destination_zones
            .contains(&forge_foundation::ZoneType::Battlefield))
    .then(|| {
        game.card(card_id)
            .svars
            .get("XPaid")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    })
}

fn leaves_battlefield_trigger(game: &GameState, sa: &SpellAbility, card_id: CardId) -> bool {
    let host = sa.trigger_source.unwrap_or(card_id);
    sa.trigger_index
        .and_then(|index| game.card(host).triggers.get(index))
        .is_some_and(|trigger| {
            trigger.mode.trigger_type() == crate::trigger::TriggerType::ChangesZone
                && trigger
                    .ir
                    .origin_zones
                    .contains(&forge_foundation::ZoneType::Battlefield)
        })
        && game.card(host).zone != forge_foundation::ZoneType::Battlefield
}

fn enters_replacement_x_paid(game: &GameState, sa: &SpellAbility, card_id: CardId) -> Option<i32> {
    sa.ir.etb.then(|| {
        game.card(card_id)
            .svars
            .get("XPaid")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    })
}

pub fn resolve_count_svar_for_sa(
    expr: &str,
    game: &GameState,
    source_id: CardId,
    controller: PlayerId,
    sa: &SpellAbility,
) -> i32 {
    use forge_foundation::ZoneType;

    // Callers reach this evaluator with expressions that never had a `Count$` prefix, so the
    // delegation in `resolve_svar_expression_inner` does not see them.
    if expr.starts_with("PlayerCount") {
        return resolve_player_count_svar(expr, game, source_id, controller, sa);
    }

    // Java `AbilityUtils.calculateAmount` walks the root ability and its sub-abilities and
    // counts every target of every node that targets; `Distinct` dedupes them. Java counts
    // targeted players too; only the cards are counted here.
    if let Some(rest) = expr.strip_prefix("TargetedObjects") {
        let distinct = rest.starts_with("Distinct");
        let rest = rest.strip_prefix("Distinct").unwrap_or(rest);
        if let Some(tail) = rest.strip_prefix('$') {
            let (property, operators) = tail.split_once('/').unwrap_or((tail, ""));
            let mut cards: Vec<CardId> = Vec::new();
            let mut current = Some(sa);
            while let Some(node) = current {
                if node.uses_targeting() {
                    for card_id in node.target_chosen.all_target_cards() {
                        if !distinct || !cards.contains(&card_id) {
                            cards.push(card_id);
                        }
                    }
                }
                current = node.sub_ability.as_deref();
            }
            let total = cards
                .iter()
                .map(|&card_id| card_x_property(card_id, property, game, source_id, controller, sa))
                .sum();
            return do_x_math(total, operators, game, source_id, controller, sa);
        }
    }

    if let Some(rest) = expr
        .strip_prefix("Count$xPaid")
        .or_else(|| expr.strip_prefix("Count$XPaid"))
    {
        let operators = rest.strip_prefix('/').unwrap_or(rest);
        let x = if sa.x_mana_cost_paid == 0 {
            enters_trigger_x_paid(game, sa, source_id)
                .or_else(|| enters_replacement_x_paid(game, sa, source_id))
                .unwrap_or(0)
        } else {
            sa.x_mana_cost_paid as i32
        };
        return do_x_math(x, operators, game, source_id, controller, sa);
    }
    if let Some(rest) = expr.strip_prefix("Count$CastTotalManaSpent ") {
        let (valid, operators) = rest.split_once('/').unwrap_or((rest, ""));
        let host = game.card(source_id);
        let spent = host
            .paying_sources_to_cast
            .iter()
            .flatten()
            .filter(|&&mana_source| {
                valid.split(',').any(|restriction| {
                    crate::card::valid_filter::matches_valid(
                        restriction,
                        Some(game.card(mana_source)),
                        None,
                        host,
                        controller,
                    )
                })
            })
            .count() as i32;
        return do_x_math(spent, operators, game, source_id, controller, sa);
    }
    if let Some(operators) = expr.strip_prefix("Count$CastTotalManaSpent") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(
            game.card(source_id).paying_mana_to_cast.len() as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }

    if expr == "Count$TriggerRememberAmount" {
        return sa.trigger_remembered_amount;
    }
    if expr == "Count$OptionalKeywordAmount" {
        return game
            .card(source_id)
            .cast_sa
            .as_ref()
            .map_or(0, |cast| cast.optional_keyword_amounts.values().sum());
    }
    if expr == "Count$ChosenNumber" {
        return game.card(source_id).chosen_number.unwrap_or(0);
    }
    if expr == "TriggerCount$Result" {
        return trigger_result_values(sa).into_iter().sum();
    }
    if expr == "TriggerCountMax$Result" {
        return trigger_result_values(sa).into_iter().max().unwrap_or(0);
    }

    if expr == "Count$Converge" || expr == "Count$Sunburst" {
        return game.card(source_id).sunburst_count();
    }

    // `Count$Adamant[_N].<Color>.<True>.<False>`: N (default 3) mana of that colour among
    // the mana spent to cast the host (`AbilityUtils`, `getPayingMana`).
    if let Some(rest) = expr.strip_prefix("Count$Adamant") {
        let parts: Vec<&str> = rest.split('.').collect();
        if let [head, color, when_true, when_false] = parts[..] {
            let needed = head
                .strip_prefix('_')
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(3);
            let atom = forge_foundation::mana::ManaAtom::from_name(&color.to_ascii_lowercase());
            let paid = game
                .card(source_id)
                .paying_mana_to_cast
                .iter()
                .filter(|&&mana| mana & atom != 0)
                .count();
            let branch = if paid >= needed {
                when_true
            } else {
                when_false
            };
            return resolve_numeric_value(game, sa, branch, 0);
        }
    }

    if let Some(operators) = expr.strip_prefix("Count$FinalChapterNr") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(
            game.card(source_id).get_final_chapter_nr(),
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }

    if expr == "Count$YourSpeed" {
        return game.player(controller).speed;
    }

    if let Some(rest) = expr.strip_prefix("Count$Domain") {
        let (needed, operators) = match rest.strip_prefix("ActivePlayer") {
            Some(operators) => (game.active_player(), operators),
            None => (controller, rest),
        };
        let lands: Vec<CardId> = game
            .cards_in_zone(ZoneType::Battlefield, needed)
            .iter()
            .copied()
            .filter(|&cid| game.card(cid).is_land() && !game.card(cid).phased_out)
            .collect();
        let n = ["Plains", "Island", "Swamp", "Mountain", "Forest"]
            .iter()
            .filter(|basic| {
                lands
                    .iter()
                    .any(|&cid| game.card(cid).type_line.has_subtype(basic))
            })
            .count() as i32;
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(n, operators, game, source_id, controller, sa);
    }

    if let Some(operators) = expr.strip_prefix("Count$Party") {
        let mut chosen_party: Vec<&str> = Vec::new();
        let mut wildcard = 0usize;
        let mut multityped: Vec<(&str, CardId)> = Vec::new();
        let mut chosen_multi: Vec<CardId> = Vec::new();
        for &cid in game.cards_in_zone(ZoneType::Battlefield, controller) {
            let card = game.card(cid);
            if !card.is_creature() {
                continue;
            }
            let creature_types: Vec<&str> = crate::card::PARTY_TYPES
                .iter()
                .copied()
                .filter(|party_type| card.has_creature_type(party_type))
                .collect();
            match creature_types.len() {
                0 => continue,
                4 => wildcard += 1,
                1 => {
                    if !chosen_party.contains(&creature_types[0]) {
                        chosen_party.push(creature_types[0]);
                    }
                }
                _ => {
                    for creature_type in creature_types {
                        multityped.push((creature_type, cid));
                    }
                }
            }
            if chosen_party.len() + wildcard >= 4 {
                break;
            }
        }

        if chosen_party.len() + wildcard < 4 {
            multityped.retain(|(creature_type, _)| !chosen_party.contains(creature_type));
            let mut groups: Vec<(&str, Vec<CardId>)> = Vec::new();
            for (creature_type, cid) in multityped {
                match groups.iter_mut().find(|(key, _)| *key == creature_type) {
                    Some((_, members)) => members.push(cid),
                    None => groups.push((creature_type, vec![cid])),
                }
            }
            groups.sort_by_key(|(creature_type, members)| (members.len(), *creature_type));
            for (creature_type, members) in groups {
                if let Some(&pick) = members.iter().find(|cid| !chosen_multi.contains(cid)) {
                    chosen_party.push(creature_type);
                    chosen_multi.push(pick);
                }
            }
        }

        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(
            (chosen_party.len() + wildcard).min(4) as i32,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }

    if let Some(operators) = expr.strip_prefix("Count$YourLifeTotal") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(
            game.player(controller).life,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }

    if let Some(operators) = expr.strip_prefix("Count$YouDrewThisTurn") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(
            game.player(controller).drawn_this_turn,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }

    if let Some(operators) = expr.strip_prefix("Count$OppGreatestLifeTotal") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        let highest_life = game
            .alive_players()
            .into_iter()
            .filter(|&pid| crate::player::player_predicates::is_opponent_of(game, controller, pid))
            .map(|pid| game.player(pid).life)
            .max()
            .unwrap_or(0);
        return do_x_math(highest_life, operators, game, source_id, controller, sa);
    }

    // Count$Metalcraft.A.B — return A if controller has 3+ artifacts, else B.
    if let Some(rest) = expr.strip_prefix("Count$Metalcraft.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let yes = parts[0].parse::<i32>().unwrap_or(1);
            let no = parts[1].parse::<i32>().unwrap_or(0);
            return if game.player_has_metalcraft(controller) {
                yes
            } else {
                no
            };
        }
    }

    if let Some(rest) = expr.strip_prefix("Count$MaxSpeed.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let yes = parts[0].parse::<i32>().unwrap_or(1);
            let no = parts[1].parse::<i32>().unwrap_or(0);
            return if game.player(controller).speed == 4 {
                yes
            } else {
                no
            };
        }
    }

    if expr == "Count$AttackersDeclared" {
        return game
            .cards
            .iter()
            .filter(|card| {
                card.controller == controller && card.attacked_this_turn && card.is_creature()
            })
            .count() as i32;
    }

    if expr == "Count$TopOfLibraryCMC" {
        return game
            .cards_in_zone(ZoneType::Library, controller)
            .last()
            .map(|&cid| game.card(cid).mana_value())
            .unwrap_or(0);
    }

    if let Some(rest) = expr.strip_prefix("Count$OptionalGenericCostPaid.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let paid_val = parts[0].parse::<i32>().unwrap_or(1);
            let unpaid_val = parts[1].parse::<i32>().unwrap_or(0);
            return if sa.optional_generic_cost_paid {
                paid_val
            } else {
                unpaid_val
            };
        }
    }

    if expr == "Count$KickedCount" {
        return sa.kick_count as i32;
    }
    if let Some(rest) = expr.strip_prefix("Count$Kicked.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let chosen = if sa.kicked { parts[0] } else { parts[1] };
            return resolve_svar_expression(chosen, game, source_id, controller, sa);
        }
    }

    // Count$UrzaLands.A.B — return A when the controller has all three Urza
    // lands, else B.
    if let Some(rest) = expr.strip_prefix("Count$UrzaLands.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let chosen = if crate::player::player_predicates::has_urza_lands(game, controller) {
                parts[0]
            } else {
                parts[1]
            };
            return resolve_svar_expression(chosen, game, source_id, controller, sa);
        }
    }

    // Count$PromisedGift.A.B — return A when gift promised, else B.
    if let Some(rest) = expr.strip_prefix("Count$PromisedGift.") {
        let parts: Vec<&str> = rest.splitn(2, '.').collect();
        if parts.len() == 2 {
            let promised_val = parts[0].parse::<i32>().unwrap_or(1);
            let not_promised_val = parts[1].parse::<i32>().unwrap_or(0);
            return if game.card(source_id).promised_gift.is_some() {
                promised_val
            } else {
                not_promised_val
            };
        }
    }
    if expr == "Count$PromisedGift" {
        return if game.card(source_id).promised_gift.is_some() {
            1
        } else {
            0
        };
    }

    // Count$Valid<Zone[,Zone...]> <restrictions>
    // Examples:
    // - Count$ValidHand Card.YouOwn
    // - Count$ValidGraveyard Card
    // - Count$ValidBattlefield Creature.YouCtrl
    if let Some(rest) = expr.strip_prefix("Count$Valid") {
        let (rest, operators) = rest.split_once('/').unwrap_or((rest, ""));
        let (zone_part, restrictions) = match rest.strip_prefix(' ') {
            Some(restrictions) => ("", restrictions.trim()),
            None => {
                let mut parts = rest.splitn(2, ' ');
                (
                    parts.next().unwrap_or("").trim(),
                    parts.next().unwrap_or("").trim(),
                )
            }
        };
        let (restrictions, aggregator) = restrictions.split_once('$').unwrap_or((restrictions, ""));
        if !restrictions.is_empty() {
            let self_only = zone_part.ends_with("Self");
            let zones: Vec<ZoneType> = if zone_part.is_empty() {
                vec![ZoneType::Battlefield]
            } else {
                zone_part
                    .split(',')
                    .filter_map(crate::ability::ability_utils::parse_zone_type)
                    .collect()
            };
            if self_only || !zones.is_empty() {
                let source = game.card(source_id);
                let selector = crate::parsing::cached_compiled_selector(restrictions);
                // Thread targets through so `TargetedPlayerOwn` etc. resolve.
                let targeted_players: Vec<crate::ids::PlayerId> =
                    sa.target_chosen.target_player.into_iter().collect();
                let targeted_cards: Vec<crate::ids::CardId> =
                    sa.target_chosen.target_card.into_iter().collect();
                let ctx = crate::card::valid_filter::MatchContext::from_source(source)
                    .with_game(game)
                    .with_source_controller(controller)
                    .with_targets(&targeted_cards, &targeted_players)
                    .with_spell_ability(sa);
                let matches: Vec<&crate::card::Card> = game
                    .cards
                    .iter()
                    .filter(|card| {
                        (if self_only {
                            card.id == source_id
                        } else {
                            zones.contains(&card.zone)
                        }) && crate::card::valid_filter::matches_valid_card_selector_with_context(
                            &selector, card, ctx,
                        )
                    })
                    .collect();
                let count = count_valid_aggregate(game, &matches, aggregator);
                return do_x_math(count, operators, game, source_id, controller, sa);
            }
        }
    }

    // Count$Valid TYPE.QUALIFIERS — count permanents matching filter
    // Count$Valid TYPE.QUALIFIERS/Times.N — count × N multiplier
    // Count$Valid TYPE.QUALIFIERS$GreatestCardPower — greatest power among matching creatures
    if let Some(filter_str) = expr.strip_prefix("Count$Valid ") {
        let (filter_str, operators) = filter_str.split_once('/').unwrap_or((filter_str, ""));
        // Check for $GreatestCardPower suffix
        let (filter_str, greatest_power) =
            if let Some(base) = filter_str.strip_suffix("$GreatestCardPower") {
                (base, true)
            } else {
                (filter_str, false)
            };

        // Check for $Colors suffix — return distinct colors among matching
        // permanents (e.g. Faeburrow Elder: Count$Valid Permanent.YouCtrl$Colors).
        let count_distinct_colors = filter_str.ends_with("$Colors");
        let filter_str = if count_distinct_colors {
            filter_str.trim_end_matches("$Colors")
        } else {
            filter_str
        };

        // Check for /Times.N multiplier suffix (e.g. "Enchantment.Other/Times.2")
        let (filter_str, multiplier) = crate::parsing::strip_times_multiplier(filter_str);

        let battlefield = game.cards_in_zone(ZoneType::Battlefield, controller);
        // Also check opponent's battlefield for non-YouCtrl filters
        let opp = game.opponent_of(controller);
        let opp_battlefield = game.cards_in_zone(ZoneType::Battlefield, opp);

        let has_you_ctrl =
            filter_str.contains(fc::YOU_CTRL) || filter_str.contains(fc::YOU_CONTROL);

        let cards_to_check: Vec<CardId> = if has_you_ctrl {
            battlefield.to_vec()
        } else {
            battlefield
                .iter()
                .chain(opp_battlefield.iter())
                .copied()
                .collect()
        };

        let source = game.card(source_id);
        let selector = crate::parsing::cached_compiled_selector(filter_str);
        let context = crate::card::valid_filter::MatchContext::from_source(source)
            .with_game(game)
            .with_source_controller(controller);
        if greatest_power {
            // Return the greatest power among matching creatures
            let mut max_power = 0;
            for &cid in &cards_to_check {
                let card = game.card(cid);
                if crate::card::valid_filter::matches_valid_card_selector_with_context(
                    &selector, card, context,
                ) {
                    max_power = max_power.max(card.power());
                }
            }
            return do_x_math(max_power, operators, game, source_id, controller, sa);
        } else if count_distinct_colors {
            let mut mask: u8 = 0;
            for &cid in &cards_to_check {
                let card = game.card(cid);
                if crate::card::valid_filter::matches_valid_card_selector_with_context(
                    &selector, card, context,
                ) {
                    mask |= card.color.mask();
                }
            }
            return do_x_math(
                (mask.count_ones() as i32) * multiplier,
                operators,
                game,
                source_id,
                controller,
                sa,
            );
        } else {
            let mut count = 0;
            for &cid in &cards_to_check {
                let card = game.card(cid);
                if crate::card::valid_filter::matches_valid_card_selector_with_context(
                    &selector, card, context,
                ) {
                    count += 1;
                }
            }
            return do_x_math(
                count * multiplier,
                operators,
                game,
                source_id,
                controller,
                sa,
            );
        }
    }

    // Count$Devotion.COLOR — count mana symbols of a color among permanents you control.
    if let Some(color_str) = expr.strip_prefix("Count$Devotion.") {
        let color_mask: u16 = match color_str.to_uppercase().as_str() {
            "W" | "WHITE" => forge_foundation::ManaAtom::WHITE,
            "U" | "BLUE" => forge_foundation::ManaAtom::BLUE,
            "B" | "BLACK" => forge_foundation::ManaAtom::BLACK,
            "R" | "RED" => forge_foundation::ManaAtom::RED,
            "G" | "GREEN" => forge_foundation::ManaAtom::GREEN,
            _ => 0,
        };
        if color_mask != 0 {
            let battlefield = game.cards_in_zone(ZoneType::Battlefield, controller);
            let mut count = 0i32;
            for &cid in battlefield {
                let card = game.card(cid);
                for shard in card.mana_cost.shards() {
                    if (shard.shard() & color_mask) != 0 {
                        count += 1;
                    }
                }
            }
            return count;
        }
    }

    // Count$Compare SVAR OPTHRESHOLD.IFTRUE.IFFALSE
    // e.g. Count$Compare Y GE1.3.1  → if Y >= 1 then 3 else 1
    if let Some(rest) = expr.strip_prefix("Count$Compare ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let svar_name = parts[0];
            let cond_parts: Vec<&str> = parts[1].splitn(3, '.').collect();
            if cond_parts.len() == 3 {
                // Resolve the referenced SVar
                let amount = |raw: &str| match game.card(source_id).get_s_var(raw) {
                    Some(svar_expr) => {
                        resolve_svar_expression(svar_expr, game, source_id, controller, sa)
                    }
                    None => resolve_svar_expression(raw, game, source_id, controller, sa),
                };
                let svar_val = amount(svar_name);
                let cond = cond_parts[0];
                let (op, rhs) = cond.split_at(cond.len().min(2));
                let result = compare_expr(svar_val, &format!("{op}{}", amount(rhs)));

                let resolve_branch = |raw: &str| {
                    raw.parse::<i32>().unwrap_or_else(|_| {
                        if let Some(svar_expr) = game.card(source_id).get_s_var(raw) {
                            resolve_svar_expression(svar_expr, game, source_id, controller, sa)
                        } else {
                            resolve_svar_expression(raw, game, source_id, controller, sa)
                        }
                    })
                };
                let if_true = resolve_branch(cond_parts[1]);
                let if_false = resolve_branch(cond_parts[2]);
                return if result { if_true } else { if_false };
            }
        }
    }

    if let Some(operators) = expr.strip_prefix("Count$ColorsColorIdentity") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        let count = game
            .player_commander_color_identity(game.card(source_id).controller)
            .len() as i32;
        return do_x_math(count, operators, game, source_id, controller, sa);
    }

    let source_left_battlefield =
        game.card(source_id).zone != forge_foundation::ZoneType::Battlefield;
    // Count$CardPower — power of the source card
    if expr == "Count$CardPower" {
        if source_left_battlefield {
            return crate::lki::resolve_lki_power(game, source_id);
        }
        return game.card(source_id).power();
    }
    // Count$CardToughness
    if expr == "Count$CardToughness" {
        if source_left_battlefield {
            return crate::lki::resolve_lki_toughness(game, source_id);
        }
        return game.card(source_id).toughness();
    }
    if let Some(operators) = expr.strip_prefix("Count$YourTurns") {
        let operators = operators.strip_prefix('/').unwrap_or(operators);
        return do_x_math(
            game.player(controller).statistics.turns_played,
            operators,
            game,
            source_id,
            controller,
            sa,
        );
    }
    // Count$CardCounters.TYPE
    if let Some(rest) = expr.strip_prefix("Count$CardCounters.") {
        let (counter_type, operators) = rest.split_once('/').unwrap_or((rest, ""));
        let use_lki = source_left_battlefield || leaves_battlefield_trigger(game, sa, source_id);
        let count = if counter_type == "ALL" {
            if use_lki {
                crate::lki::resolve_lki_counters(game, source_id)
                    .iter()
                    .map(|(_, count)| count)
                    .sum()
            } else {
                game.card(source_id).num_all_counters()
            }
        } else {
            let ct = crate::ability::effects::parse_counter_type(counter_type);
            if use_lki {
                crate::lki::resolve_lki_counter_count(game, source_id, &ct)
            } else {
                *game.card(source_id).counters.get(&ct).unwrap_or(&0)
            }
        };
        return do_x_math(count, operators, game, source_id, controller, sa);
    }

    // Count$TotalDamageDoneByThisTurn — total damage dealt by the source card this turn.
    if expr == "Count$TotalDamageDoneByThisTurn" {
        return game.card(source_id).total_damage_done_this_turn;
    }

    // Count$InYour<Zone> / Count$CardsInYour<Zone> — zone size for the SA's
    // controller (e.g. `Count$CardsInYourHand` returns the hand size of the
    // ability's "you"). Mirrors Java `AbilityUtils.getCardListForXCount`'s
    // `InYour<Zone>` substring branch (`AbilityUtils.java:3718`).
    if let Some(rest) = expr
        .strip_prefix("Count$CardsInYour")
        .or_else(|| expr.strip_prefix("Count$InYour"))
    {
        let zone = match rest {
            "Hand" => Some(ZoneType::Hand),
            "Yard" | "Graveyard" => Some(ZoneType::Graveyard),
            "Library" => Some(ZoneType::Library),
            "Exile" => Some(ZoneType::Exile),
            "Battlefield" => Some(ZoneType::Battlefield),
            _ => None,
        };
        if let Some(zone) = zone {
            return game.cards_in_zone(zone, controller).len() as i32;
        }
    }

    if let Some(rest) = expr.strip_prefix("Count$MaxSameStoredRolls") {
        let operators = rest.strip_prefix('/').unwrap_or(rest);
        let mut max_count = 0;
        let mut current_count = 0;
        let mut previous = None;
        for roll in &game.card(source_id).stored_rolls {
            if previous == Some(*roll) {
                current_count += 1;
            } else {
                previous = Some(*roll);
                current_count = 1;
            }
            max_count = max_count.max(current_count);
        }
        return do_x_math(max_count, operators, game, source_id, controller, sa);
    }

    if let Some(rest) = expr.strip_prefix("Count$RememberedNumber") {
        let operators = rest.strip_prefix('/').unwrap_or(rest);
        let count = game.card(source_id).remembered_cmc.iter().sum();
        return do_x_math(count, operators, game, source_id, controller, sa);
    }

    // Count$RememberedSize — mirrors Java `Card.getRememberedCount()`
    // (cards + players + integers).
    if let Some(rest) = expr.strip_prefix("Count$RememberedSize") {
        let operators = rest.strip_prefix('/').unwrap_or(rest);
        let card = game.card(source_id);
        let count = card.remembered_cards.len()
            + card.remembered_players.len()
            + card.remembered_cmc.len()
            + card.remembered_counters.len();
        return do_x_math(count as i32, operators, game, source_id, controller, sa);
    }

    if let Some(body) = expr.strip_prefix("Count$") {
        let (l0, operators) = body.split_once('/').unwrap_or((body, ""));
        let sq: Vec<&str> = l0.split('.').collect();
        let paidparts: Vec<&str> = l0.splitn(2, '$').collect();
        let player = game.player(controller);
        let math = |num: i32| do_x_math(num, operators, game, source_id, controller, sa);
        let calculate_branch = |condition: bool| {
            let chosen = sq
                .get(if condition { 1 } else { 2 })
                .copied()
                .unwrap_or("0");
            resolve_svar_expression(chosen, game, source_id, controller, sa)
        };

        if sq[0].starts_with("IsPrime") {
            let comp_string: Vec<&str> = sq[0].split(' ').collect();
            let lhs = resolve_svar_expression(
                comp_string.get(1).copied().unwrap_or("0"),
                game,
                source_id,
                controller,
                sa,
            );
            let v = lhs > 1 && (2..).take_while(|d| d * d <= lhs).all(|d| lhs % d != 0);
            return math(calculate_branch(v));
        }
        if sq[0] == "ResolvedThisTurn" {
            let host = sa.source.unwrap_or(source_id);
            return math(game.card(host).get_ability_resolved_this_turn(Some(sa)) as i32);
        }
        if sq[0] == "Delirium" {
            return math(calculate_branch(game.player_has_delirium(controller)));
        }
        if sq[0] == "CommittedCrimeThisTurn" {
            return math(calculate_branch(player.committed_crime_this_turn > 0));
        }
        if sq[0] == "YourStartingLife" {
            return math(player.starting_life);
        }
        if sq[0].contains("LifeYouLostThisTurn") {
            return math(player.life_lost_this_turn);
        }
        if sq[0].contains("LifeYouGainedThisTurn") {
            return math(player.life_gained_this_turn);
        }
        if sq[0].contains("LifeYourTeamGainedThisTurn") {
            return math(player.life_gained_by_team_this_turn);
        }
        if sq[0].contains("LifeYouGainedTimesThisTurn") {
            return math(player.life_gained_times_this_turn);
        }
        if sq[0].contains("LifeOppsLostThisTurn") {
            let lost = game
                .player_order
                .iter()
                .filter(|&&pid| {
                    crate::player::player_predicates::is_opponent_of(game, controller, pid)
                })
                .map(|&pid| game.player(pid).life_lost_this_turn)
                .sum();
            return math(lost);
        }
        if sq[0] == "CrewSize" {
            return math(game.card(source_id).crewed_by_this_turn.len() as i32);
        }
        if sq[0] == "TotalDamageReceivedThisTurn" {
            return math(game.card(source_id).assigned_damage);
        }
        if sq[0] == "MaxCombatDamageThisTurn" {
            return math(
                game.players
                    .iter()
                    .map(|p| p.combat_damage_received_this_turn)
                    .max()
                    .unwrap_or(0),
            );
        }
        if sq[0].starts_with("Morbid") {
            let res = crate::card::card_util::get_this_turn_entered(
                game,
                ZoneType::Graveyard,
                Some(ZoneType::Battlefield),
                "Creature",
                source_id,
                controller,
            );
            return math(calculate_branch(!res.is_empty()));
        }

        if sq[0].starts_with("CountersAddedThisTurn") {
            let parts: Vec<&str> = l0.split(' ').collect();
            if parts.len() >= 4 {
                let counter_type = (!parts[1].eq_ignore_ascii_case("Any"))
                    .then(|| crate::ability::effects::parse_counter_type(parts[1]));
                return math(game.get_counter_added_this_turn(
                    counter_type.as_ref(),
                    parts[2],
                    parts[3],
                    source_id,
                    controller,
                ));
            }
        }
        if sq[0].contains("TimesKicked") {
            let card = game.card(source_id);
            let magnitude = if crate::ability::ability_utils::is_unlinked_from_cast_sa(sa, card) {
                0
            } else {
                card.cast_sa.as_ref().map_or(0, |cast_sa| {
                    if cast_sa.kick_count > 0 {
                        cast_sa.kick_count as i32
                    } else {
                        let has_k1 = cast_sa
                            .optional_costs
                            .contains(&crate::spellability::OptionalCost::Kicker1);
                        let has_k2 = cast_sa
                            .optional_costs
                            .contains(&crate::spellability::OptionalCost::Kicker2);
                        if has_k1 == has_k2 {
                            if has_k1 {
                                2
                            } else {
                                0
                            }
                        } else {
                            1
                        }
                    }
                })
            };
            return math(magnitude);
        }
        if sq[0].starts_with("Bargain") {
            return math(calculate_branch(
                sa.optional_costs
                    .contains(&crate::spellability::OptionalCost::Bargain),
            ));
        }
        if sq[0].starts_with("Teamwork") {
            return math(calculate_branch(
                sa.optional_costs
                    .contains(&crate::spellability::OptionalCost::Teamwork),
            ));
        }
        if sq[0] == "YouFlipThisTurn" {
            return math(game.player(controller).num_flips_this_turn);
        }
        if sq[0] == "YouDescendedThisTurn" {
            return math(
                game.player(controller)
                    .permanents_put_into_graveyard_this_turn,
            );
        }
        if sq[0] == "UnlockedDoors" {
            let doors = game
                .cards_in_zone(ZoneType::Battlefield, controller)
                .iter()
                .map(|&card| game.card(card))
                .filter(|card| card.type_line.has_subtype("Room"))
                .map(|card| card.get_unlocked_room_count())
                .sum();
            return math(doors);
        }
        if sq[0].starts_with("Void") {
            return math(calculate_branch(game.is_void()));
        }
        if sq[0].starts_with("LeftBattlefieldThisTurn")
            || sq[0].starts_with("LeftGraveyardThisTurn")
        {
            if let Some((_, valid_filter)) = l0.split_once(' ') {
                let selector = crate::parsing::cached_compiled_selector(valid_filter);
                let source = game.card(source_id);
                let list = if sq[0].starts_with("LeftBattlefieldThisTurn") {
                    &game.left_battlefield_this_turn
                } else {
                    &game.left_graveyard_this_turn
                };
                let count = list
                    .iter()
                    .filter(|&&card| {
                        crate::card::valid_filter::matches_valid_card_selector_in_game(
                            &selector,
                            game.card(card),
                            source,
                            game,
                        )
                    })
                    .count();
                return math(count as i32);
            }
        }
        if sq[0].starts_with("ImprintedSize") {
            return math(game.card(source_id).imprinted_cards.len() as i32);
        }
        if let Some(rest) = sq[0].strip_prefix("wasCastFrom") {
            let your = sq[0].contains("Your");
            let by_you = sq[0].contains("ByYou");
            let mut str_zone = rest;
            if your {
                str_zone = str_zone.strip_prefix("Your").unwrap_or(str_zone);
            }
            if by_you {
                str_zone = &str_zone[..str_zone.find("ByYou").unwrap_or(str_zone.len())];
            }
            let card = game.card(source_id);
            let zones_match = card
                .cast_from
                .is_some_and(|zone| Some(zone) == ZoneType::from_str_compat(str_zone))
                && (!by_you
                    || card
                        .cast_sa
                        .as_ref()
                        .is_some_and(|cast_sa| cast_sa.activating_player == controller))
                && (!your || card.owner == controller);
            return math(calculate_branch(zones_match));
        }
        if sq[0].ends_with("InOwnMainPhase") {
            let is_my_main = game.turn.is_main_phase()
                && game.active_player() == controller
                && (!sq[0].starts_with("IfCast") || game.card(source_id).was_cast());
            return math(
                sq.get(if is_my_main { 1 } else { 2 })
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(0),
            );
        }
        if sq[0].starts_with("FinishedUpkeepsThisTurn") {
            return math(
                game.turn.n_upkeeps_this_turn
                    - i32::from(game.turn.phase == forge_foundation::PhaseType::Upkeep),
            );
        }
        if sq[0].starts_with("FinishedEndOfTurnsThisTurn") {
            return math(
                game.turn.n_end_of_turns_this_turn
                    - i32::from(game.turn.phase == forge_foundation::PhaseType::EndOfTurn),
            );
        }
        if sq[0].starts_with("CreaturesAttackedThisTurn") {
            if let Some((_, valid_filter)) = l0.split_once(' ') {
                let selector = crate::parsing::cached_compiled_selector(valid_filter);
                let source = game.card(source_id);
                let count = game
                    .cards
                    .iter()
                    .filter(|card| {
                        card.controller == controller
                            && card.attacked_this_turn
                            && card.is_creature()
                    })
                    .filter(|card| {
                        crate::card::valid_filter::matches_valid_card_selector_in_game(
                            &selector, card, source, game,
                        )
                    })
                    .count();
                return math(count as i32);
            }
        }
        if sq[0].starts_with("MostProminentCreatureType") {
            if let Some((_, restriction)) = l0.split_once(' ') {
                let selector = crate::parsing::cached_compiled_selector(restriction);
                let source = game.card(source_id);
                let mut all_creature_type = 0;
                let mut map: crate::HashMap<&str, i32> = crate::HashMap::default();
                for card in game.cards.iter().filter(|card| {
                    card.zone == ZoneType::Battlefield
                        && crate::card::valid_filter::matches_valid_card_selector_in_game(
                            &selector, card, source, game,
                        )
                }) {
                    if card.has_keyword("Changeling") {
                        all_creature_type += 1;
                        continue;
                    }
                    for creature_type in card
                        .type_line
                        .subtypes
                        .iter()
                        .filter(|subtype| crate::game::TypeRegistry::is_creature_type(subtype))
                    {
                        *map.entry(creature_type.as_str()).or_default() += 1;
                    }
                }
                return math(map.values().copied().max().unwrap_or(0) + all_creature_type);
            }
        }
        if let Some(rest) = l0.strip_prefix("DifferentCounterKinds_") {
            let selector = crate::parsing::cached_compiled_selector(rest);
            let source = game.card(source_id);
            let kinds: std::collections::BTreeSet<_> = game
                .cards
                .iter()
                .filter(|card| {
                    card.zone == ZoneType::Battlefield
                        && crate::card::valid_filter::matches_valid_card_selector_in_game(
                            &selector, card, source, game,
                        )
                })
                .flat_map(|card| {
                    card.counters
                        .iter()
                        .filter(|(_, amount)| **amount > 0)
                        .map(|(counter_type, _)| counter_type.clone())
                })
                .collect();
            return math(kinds.len() as i32);
        }

        let mut some_cards: Option<Vec<CardId>> = None;
        if sq[0].starts_with("LastStateBattlefieldWithFallback") {
            if let Some((_, valid)) = paidparts[0].split_once(' ') {
                let host = game.card(sa.source.unwrap_or(source_id));
                if !host.was_cast() {
                    return math(0);
                }
                let mut cards = host
                    .cast_sa
                    .as_ref()
                    .map_or_else(Vec::new, |cast_sa| cast_sa.last_state_battlefield.clone());
                if cards.is_empty() {
                    cards = game
                        .cards
                        .iter()
                        .filter(|card| card.zone == ZoneType::Battlefield)
                        .map(|card| card.id)
                        .collect();
                }
                let selector = crate::parsing::cached_compiled_selector(valid);
                let targeted_players: Vec<crate::ids::PlayerId> =
                    sa.target_chosen.target_player.into_iter().collect();
                let targeted_cards: Vec<crate::ids::CardId> =
                    sa.target_chosen.target_card.into_iter().collect();
                let ctx =
                    crate::card::valid_filter::MatchContext::from_source(game.card(source_id))
                        .with_game(game)
                        .with_targets(&targeted_cards, &targeted_players)
                        .with_spell_ability(sa);
                some_cards = Some(
                    cards
                        .into_iter()
                        .filter(|&card| {
                            crate::card::valid_filter::matches_valid_card_selector_with_context(
                                &selector,
                                game.card(card),
                                ctx,
                            )
                        })
                        .collect(),
                );
            }
        }
        if sq[0].starts_with("ThisTurnCast") || sq[0].starts_with("LastTurnCast") {
            let working_copy: Vec<&str> = paidparts[0].split('_').collect();
            if let Some(&valid_filter) = working_copy.get(1) {
                some_cards = Some(if working_copy[0].contains("This") {
                    crate::card::card_util::get_this_turn_cast(
                        game,
                        valid_filter,
                        source_id,
                        controller,
                    )
                } else {
                    crate::card::card_util::get_last_turn_cast(
                        game,
                        valid_filter,
                        source_id,
                        controller,
                    )
                });
            }
        }
        if sq[0].starts_with("ThisTurnActivated") {
            let working_copy: Vec<&str> = paidparts[0].split('_').collect();
            if let Some(&valid_filter) = working_copy.get(1) {
                let activated = crate::card::card_util::get_this_turn_activated(
                    game,
                    valid_filter,
                    source_id,
                    controller,
                )
                .len();
                return math(activated as i32);
            }
        }
        if sq[0].starts_with("ThisTurnEntered") || sq[0].starts_with("LastTurnEntered") {
            let working_copy: Vec<&str> = paidparts[0].splitn(5, '_').collect();
            let destination = working_copy
                .get(1)
                .and_then(|zone| ZoneType::from_str_compat(zone));
            let has_from = working_copy.get(2) == Some(&"from");
            let origin = if has_from {
                working_copy
                    .get(3)
                    .and_then(|zone| ZoneType::from_str_compat(zone))
            } else {
                None
            };
            if let (Some(destination), Some(&valid_filter)) =
                (destination, working_copy.get(if has_from { 4 } else { 2 }))
            {
                some_cards = Some(if sq[0].starts_with("This") {
                    crate::card::card_util::get_this_turn_entered(
                        game,
                        destination,
                        origin,
                        valid_filter,
                        source_id,
                        controller,
                    )
                } else {
                    crate::card::card_util::get_last_turn_entered(
                        game,
                        destination,
                        origin,
                        valid_filter,
                        source_id,
                        controller,
                    )
                });
            }
        }
        if let Some(some_cards) = some_cards {
            let num = match paidparts.get(1) {
                Some(property) => crate::ability::ability_utils::handle_paid(
                    game,
                    &some_cards,
                    property,
                    source_id,
                ),
                None => some_cards.len() as i32,
            };
            return math(num);
        }
    }

    expr.parse::<i32>().unwrap_or_else(|_| {
        crate::census::unhandled("count-expression-as-zero", expr);
        eprintln!("Unrecognized Count expression, returning 0 for: {expr}");
        0
    })
}

/// Check if a card matches a validity filter string like "Forest.YouCtrl".
#[allow(dead_code)]
fn valid_card_matches_with_source(
    filter: &str,
    card: &crate::card::Card,
    controller: PlayerId,
    source_id: CardId,
    chosen_type: Option<&str>,
) -> bool {
    let parts: Vec<&str> = filter.split('.').collect();
    let base_type = parts.first().copied().unwrap_or("");

    // Check base type
    let type_ok = match base_type {
        fc::CREATURE => card.is_creature(),
        fc::LAND => card.is_land(),
        fc::ARTIFACT => card.type_line.is_artifact(),
        fc::ENCHANTMENT => card.type_line.is_enchantment(),
        fc::PLANESWALKER => card.type_line.is_planeswalker(),
        fc::PERMANENT | fc::CARD => true,
        // Subtypes (Forest, Island, Goblin, etc.)
        _ => card.type_line.has_subtype(base_type),
    };
    if !type_ok {
        return false;
    }

    // Check qualifiers (split by '.' and '+')
    for &dot_qual in &parts[1..] {
        for sub_qual in dot_qual.split('+') {
            let sub_qual = sub_qual.trim();
            if sub_qual.eq_ignore_ascii_case(fc::YOU_CTRL)
                || sub_qual.eq_ignore_ascii_case(fc::YOU_CONTROL)
            {
                if card.controller != controller {
                    return false;
                }
            } else if sub_qual.eq_ignore_ascii_case(fc::SELF_REF) {
                if card.id != source_id {
                    return false;
                }
            } else if sub_qual.eq_ignore_ascii_case(fc::OTHER) {
                if card.id == source_id {
                    return false;
                }
            } else if sub_qual.eq_ignore_ascii_case("ChosenType") {
                // Card must have the source card's chosen creature type as a subtype.
                // Changeling means all creature types — always matches.
                match chosen_type {
                    Some(ct)
                        if card.type_line.has_subtype(ct) || card.has_keyword("Changeling") => {}
                    _ => return false,
                }
            } else if sub_qual.starts_with("counters_") {
                // Parse "counters_GE1_P1P1", "counters_EQ0_P1P1", etc.
                if !check_counter_qualifier(card, sub_qual) {
                    return false;
                }
            }
        }
    }
    true
}

/// Check a counter qualifier like "counters_GE1_P1P1".
#[allow(dead_code)]
fn check_counter_qualifier(card: &crate::card::Card, qual: &str) -> bool {
    let rest = match qual.strip_prefix("counters_") {
        Some(r) => r,
        None => return true,
    };
    // Split into OP+THRESHOLD and COUNTER_TYPE, e.g. "GE1_P1P1"
    let parts: Vec<&str> = rest.splitn(2, '_').collect();
    if parts.len() != 2 {
        return true;
    }
    let cond = parts[0];
    let counter_type = crate::ability::effects::parse_counter_type(parts[1]);
    let count = *card.counters.get(&counter_type).unwrap_or(&0);

    compare_expr(count, cond)
}

#[cfg(test)]
mod tests {
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost};

    use super::resolve_numeric_svar;
    use crate::card::Card;
    use crate::game::GameState;
    use crate::ids::{CardId, PlayerId};
    use crate::spellability::SpellAbility;

    #[test]
    fn resolves_player_count_defined_life_total_twice() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        game.player_mut(p1).life = 7;

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "PlayerCountDefinedTriggeredAttackedTarget$LifeTotal/Twice".to_string(),
        );
        let host_id = game.create_card(host);

        let mut sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ GainLife | LifeAmount$ X");
        sa.set_triggering_value(
            crate::ability::AbilityKey::AttackedTarget,
            crate::event::AbilityValue::Player(p1),
        );

        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 14);
    }

    #[test]
    fn resolves_player_count_highest_life_total() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        game.player_mut(p0).life = 11;
        game.player_mut(p1).life = 17;

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "PlayerCountPlayers$HighestLifeTotal".to_string(),
        );
        let host_id = game.create_card(host);

        let sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ GainLife | LifeAmount$ X");
        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 17);
    }

    #[test]
    fn resolves_triggered_target_life_total_half_up() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        game.player_mut(p1).life = 9;

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "TriggeredTarget$LifeTotal/HalfUp".to_string(),
        );
        let host_id = game.create_card(host);

        let mut sa = SpellAbility::new_simple(
            Some(host_id),
            p0,
            "DB$ LoseLife | Defined$ TriggeredTarget | LifeAmount$ X",
        );
        sa.set_triggering_value(
            crate::ability::AbilityKey::TargetPlayer,
            crate::event::AbilityValue::Player(p1),
        );

        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 5);
    }

    #[test]
    fn resolves_player_count_minus_remembered_amount() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        let remembered = Card::new(
            CardId(1),
            "Remembered".to_string(),
            p1,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        let remembered_id = game.create_card(remembered);

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "PlayerCountOpponents$Amount/Minus.Remembered$Amount".to_string(),
        );
        let host_id = game.create_card(host);
        game.card_mut(host_id).add_remembered_card(remembered_id);

        let sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ Draw | NumCards$ X");
        assert_eq!(resolve_numeric_svar(&game, &sa, "NumCards", -1), 0);
    }

    #[test]
    fn resolves_player_count_minus_empty_remembered_amount() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "PlayerCountOpponents$Amount/Minus.Remembered$Amount".to_string(),
        );
        let host_id = game.create_card(host);

        let sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ Draw | NumCards$ X");
        assert_eq!(resolve_numeric_svar(&game, &sa, "NumCards", -1), 1);
    }

    #[test]
    fn resolves_player_count_remembered_life_lost_this_turn() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        game.player_mut(p1).life_lost_this_turn = 11;

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "PlayerCountRemembered$LifeLostThisTurn".to_string(),
        );
        let host_id = game.create_card(host);
        game.card_mut(host_id).add_remembered_player(p1);

        let sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ LoseLife | LifeAmount$ X");
        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", -1), 11);
    }

    #[test]
    fn resolves_triggered_spell_ability_card_mana_cost_lki() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "X".to_string(),
            "TriggeredSpellAbility$CardManaCostLKI".to_string(),
        );
        let host_id = game.create_card(host);

        let mut spell_card = Card::new(
            CardId(1),
            "Big Spell".to_string(),
            p1,
            CardTypeLine::parse("Sorcery"),
            ManaCost::parse("X U"),
            ColorSet::BLUE,
            None,
            None,
            vec![],
            vec![],
        );
        spell_card.set_zone(forge_foundation::ZoneType::Graveyard);
        let spell_id = game.create_card(spell_card);

        let mut triggered_sa =
            SpellAbility::new_simple(Some(spell_id), p1, "SP$ DealDamage | NumDmg$ 1");
        triggered_sa.x_mana_cost_paid = 4;

        let mut sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ GainLife | LifeAmount$ X");
        sa.set_triggering_spell_ability("SpellAbility", triggered_sa);

        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 5);
    }

    #[test]
    fn resolves_count_your_speed_and_max_speed() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        game.player_mut(p0).speed = 4;

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars
            .insert("X".to_string(), "Count$YourSpeed".to_string());
        host.svars
            .insert("Y".to_string(), "Count$MaxSpeed.2.1".to_string());
        let host_id = game.create_card(host);

        let sa = SpellAbility::new_simple(
            Some(host_id),
            p0,
            "DB$ GainLife | LifeAmount$ X | NumCards$ Y",
        );
        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 4);
        assert_eq!(resolve_numeric_svar(&game, &sa, "NumCards", 0), 2);
    }

    #[test]
    fn resolves_attackers_declared_and_life_lost_last_turn() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);

        let mut attacker = Card::new(
            CardId(0),
            "Attacker".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse("1 R"),
            ColorSet::RED,
            Some(2),
            Some(2),
            vec![],
            vec![],
        );
        attacker.attacked_this_turn = true;
        game.create_card(attacker);

        game.player_mut(p0).life_lost_this_turn = 3;
        game.player_mut(p0).new_turn();

        let mut host = Card::new(
            CardId(1),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars
            .insert("X".to_string(), "Count$AttackersDeclared".to_string());
        host.svars.insert(
            "Y".to_string(),
            "PlayerCountPropertyYou$LifeLostLastTurn".to_string(),
        );
        let host_id = game.create_card(host);

        let sa = SpellAbility::new_simple(
            Some(host_id),
            p0,
            "DB$ GainLife | LifeAmount$ X | NumCards$ Y",
        );
        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 1);
        assert_eq!(resolve_numeric_svar(&game, &sa, "NumCards", 0), 3);
    }

    #[test]
    fn resolves_top_of_library_cmc() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);

        let top = Card::new(
            CardId(0),
            "Top".to_string(),
            p0,
            CardTypeLine::parse("Sorcery"),
            ManaCost::parse("2 U"),
            ColorSet::BLUE,
            None,
            None,
            vec![],
            vec![],
        );
        let top_id = game.create_card(top);
        game.move_card(top_id, forge_foundation::ZoneType::Library, p0);

        let mut host = Card::new(
            CardId(1),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars
            .insert("X".to_string(), "Count$TopOfLibraryCMC".to_string());
        let host_id = game.create_card(host);

        let sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ GainLife | LifeAmount$ X");
        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 3);
    }

    #[test]
    fn resolves_player_property_counters_for_discard_damage_and_combat() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        game.player_mut(p0).discarded_this_turn = 2;
        game.player_mut(p0).explored_this_turn = 1;
        game.player_mut(p0).opponents_assigned_damage_this_turn = 4;
        game.player_mut(p0).assigned_damage_this_turn = 7;
        game.player_mut(p0).assigned_combat_damage_this_turn = 2;
        game.player_mut(p0).attacked_players_this_combat.push(p1);
        game.player_mut(p0).been_dealt_combat_damage_since_last_turn = true;

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars.insert(
            "A".to_string(),
            "PlayerCountPropertyYou$CardsDiscardedThisTurn".to_string(),
        );
        host.svars.insert(
            "B".to_string(),
            "PlayerCountPropertyYou$ExploredThisTurn".to_string(),
        );
        host.svars.insert(
            "C".to_string(),
            "PlayerCountPropertyYou$DamageToOppsThisTurn".to_string(),
        );
        host.svars.insert(
            "D".to_string(),
            "PlayerCountPropertyYou$NonCombatDamageDealtThisTurn".to_string(),
        );
        host.svars.insert(
            "E".to_string(),
            "PlayerCountPropertyYou$OpponentsAttackedThisCombat".to_string(),
        );
        host.svars.insert(
            "F".to_string(),
            "PlayerCountPropertyYou$BeenDealtCombatDamageSinceLastTurn".to_string(),
        );
        let host_id = game.create_card(host);

        let sa = SpellAbility::new_simple(
            Some(host_id),
            p0,
            "DB$ GainLife | LifeAmount$ A | NumCards$ B",
        );
        assert_eq!(resolve_numeric_svar(&game, &sa, "LifeAmount", 0), 2);
        assert_eq!(resolve_numeric_svar(&game, &sa, "NumCards", 0), 1);
        assert_eq!(
            super::resolve_svar_expression(
                game.card(host_id).get_s_var("C").unwrap(),
                &game,
                host_id,
                p0,
                &sa,
            ),
            4
        );
        assert_eq!(
            super::resolve_svar_expression(
                game.card(host_id).get_s_var("D").unwrap(),
                &game,
                host_id,
                p0,
                &sa,
            ),
            5
        );
        assert_eq!(
            super::resolve_svar_expression(
                game.card(host_id).get_s_var("E").unwrap(),
                &game,
                host_id,
                p0,
                &sa,
            ),
            1
        );
        assert_eq!(
            super::resolve_svar_expression(
                game.card(host_id).get_s_var("F").unwrap(),
                &game,
                host_id,
                p0,
                &sa,
            ),
            1
        );
    }

    #[test]
    fn resolves_trigger_result_sum_and_max_from_trigger_objects() {
        let mut game = GameState::new(&["A", "B"], 20);
        let p0 = PlayerId(0);

        let mut host = Card::new(
            CardId(0),
            "Host".to_string(),
            p0,
            CardTypeLine::parse("Creature"),
            ManaCost::parse(""),
            ColorSet::COLORLESS,
            Some(1),
            Some(1),
            vec![],
            vec![],
        );
        host.svars
            .insert("Sum".to_string(), "TriggerCount$Result".to_string());
        host.svars
            .insert("Max".to_string(), "TriggerCountMax$Result".to_string());
        let host_id = game.create_card(host);

        let mut sa = SpellAbility::new_simple(Some(host_id), p0, "DB$ Draw | NumCards$ Sum");
        sa.set_triggering_object(crate::ability::AbilityKey::Result, "4,11,7");

        assert_eq!(
            super::resolve_svar_expression(
                game.card(host_id).get_s_var("Sum").unwrap(),
                &game,
                host_id,
                p0,
                &sa,
            ),
            22
        );
        assert_eq!(
            super::resolve_svar_expression(
                game.card(host_id).get_s_var("Max").unwrap(),
                &game,
                host_id,
                p0,
                &sa,
            ),
            11
        );
    }
}
