//! Mirrors Java's `SpellAbilityProperty`.

use forge_foundation::ZoneType;

use crate::ability::api_type::ApiType;
use crate::card::valid_filter::MatchContext;
use crate::card::{Card, CounterType};
use crate::ids::{CardId, PlayerId};
use crate::keyword::Keyword;
use crate::spellability::{AlternativeCost, OptionalCost, SpellAbility};
use crate::trigger::TriggerType;

pub fn has_property(
    sa: &SpellAbility,
    property: &str,
    ability_host: Option<&Card>,
    context: MatchContext<'_>,
) -> bool {
    let source = context.source_card;
    let source_controller = context.source_controller;
    let game = context.game;
    match property {
        "ManaAbility" => is_mana_ability(sa, context),
        "withoutXCost" => !sa.cost_has_mana_x(),
        "hasTapCost" => sa
            .pay_costs
            .as_ref()
            .is_some_and(|cost| cost.has_tap_cost()),
        "Bargain" => sa.is_optional_cost_paid(OptionalCost::Bargain),
        "Backup" => sa.is_keyword(Keyword::Backup),
        "Bestow" => sa.is_alternative_cost(AlternativeCost::Bestow),
        "Blitz" => sa.is_alternative_cost(AlternativeCost::Blitz),
        "Buyback" => sa.is_buyback(),
        "Craft" => sa.is_craft(),
        "Crew" => sa.is_keyword(Keyword::Crew),
        "Saddle" => sa.is_keyword(Keyword::Saddle),
        "Station" => sa.is_keyword(Keyword::Station),
        "Cycling" => sa.is_cycling(),
        "Dash" => sa.is_alternative_cost(AlternativeCost::Dash),
        "Disturb" => sa.is_alternative_cost(AlternativeCost::Disturb),
        "Embalm" => sa.is_keyword(Keyword::Embalm),
        "Eternalize" => sa.is_keyword(Keyword::Eternalize),
        "BeamMeUp" => false,
        "Flashback" => sa.is_alternative_cost(AlternativeCost::Flashback),
        "Harmonize" => sa.is_alternative_cost(AlternativeCost::Harmonize),
        "Jumpstart" => sa.is_optional_cost_paid(OptionalCost::Jumpstart),
        "Kicked" => sa.is_kicked(),
        "Loyalty" => sa.is_pw_ability(),
        "Aftermath" => false,
        "PowerUp" => sa.has_param("PowerUp"),
        "MorphUp" => sa.has_param("MorphUp"),
        "ManifestUp" => sa.has_param("ManifestUp"),
        "Teamwork" => sa.is_optional_cost_paid(OptionalCost::Teamwork),
        "Unlock" => sa.is_unlock(),
        "isTurnFaceUp" => sa.is_turn_face_up(),
        "isCastFaceDown" => sa.cast_face_down,
        "Unearth" => sa.is_keyword(Keyword::Unearth),
        "Modular" => sa.is_keyword(Keyword::Modular),
        "Equip" => sa.is_keyword(Keyword::Equip),
        "Boast" => sa.has_param("Boast"),
        "Monstrosity" => sa.has_param("Monstrosity"),
        "Exhaust" => sa.has_param("Exhaust"),
        "Mayhem" => sa.is_alternative_cost(AlternativeCost::Mayhem),
        "Mutate" => sa.is_alternative_cost(AlternativeCost::Mutate),
        "Ninjutsu" => sa.is_keyword(Keyword::Ninjutsu),
        "Sneak" => sa.is_alternative_cost(AlternativeCost::Sneak),
        "Foretelling" => false,
        "Foretold" => sa.is_alternative_cost(AlternativeCost::Foretell),
        "Plotting" => sa.is_plotting(),
        "Outlast" => sa.is_keyword(Keyword::Outlast),
        "Modal" => sa.api == Some(ApiType::Charm),
        "ClassLevelUp" => sa.api == Some(ApiType::ClassLevelUp),
        "Daybound" => sa.is_keyword(Keyword::Daybound),
        "Nightbound" => sa.is_keyword(Keyword::Nightbound),
        "Warp" => sa.is_alternative_cost(AlternativeCost::Warp),
        "Ward" => sa.is_keyword(Keyword::Ward),
        "CumulativeUpkeep" => sa.has_param("CumulativeUpkeep"),
        // Java compares the KeywordInterface instances; Rust tags an ability with its keyword
        // kind only.
        "SameKeyword" => sa.get_keyword().is_some_and(|keyword| {
            context
                .spell_ability
                .is_some_and(|ctb| ctb.get_keyword() == Some(keyword))
        }),
        "ChapterNotLore" => {
            sa.is_chapter(game)
                && sa
                    .get_trigger(game)
                    .and_then(|trigger| trigger.get_chapter())
                    != ability_host.map(|host| host.counter_count(&CounterType::Lore))
        }
        "EffectSourceAbility" => false,
        "LastChapter" => sa.is_last_chapter(game),
        "paidPhyrexianMana" => false,
        "MayPlaySource" => sa.may_play_source == Some(source.id),
        "YouCtrl" => sa.activating_player == source_controller,
        "OppCtrl" => crate::player::player_predicates::is_opponent_of(
            game,
            sa.activating_player,
            source_controller,
        ),
        "ManaAbilityCantPaidFor" => false,
        "NamedSpell" => ability_host.is_some_and(|host| {
            let name = sa.card_state_name(host);
            source.named_cards.iter().any(|named| named == name)
        }),
        "otherAbility" => context.spell_ability.is_none_or(|ctb| ctb.id != sa.id),
        "CouldCastTiming" => could_cast_timing(sa, ability_host, context),
        _ => {
            if let Some(rest) = property.strip_prefix("XCost") {
                let (comparator, amount) = split_comparator(rest);
                let y = crate::svar::resolve_numeric_value(game, sa, amount, 0);
                compare(sa.x_mana_cost_paid as i32, comparator, y)
            } else if let Some(rest) = property.strip_prefix("CountersRemovedToPay") {
                let (comparator, amount) = split_comparator(rest);
                let y = crate::svar::resolve_numeric_value(game, sa, amount, 0);
                let removed = sa
                    .svars
                    .get("CostCountersRemoved")
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
                compare(removed, comparator, y)
            } else if let Some(defined) = property.strip_prefix("ManaSpentBy ") {
                let spenders = defined_players(defined, context);
                ability_host.is_some_and(|host| !host.paying_mana_to_cast.is_empty())
                    && spenders.contains(&sa.activating_player)
            } else if let Some(rest) = property.strip_prefix("ManaSpent ") {
                let spent = ability_host.map_or(0, |host| host.paying_mana_to_cast.len() as i32);
                crate::card::valid_filter::compare_raw_operand(spent, rest, Some(context))
            } else if let Some(from_what) = property.strip_prefix("ManaFrom") {
                mana_from(from_what, sa, ability_host, context)
            } else if property.starts_with("singleTarget") {
                single_target(sa)
            } else if let Some(rest) = property.strip_prefix("numTargets ") {
                let (comparator, amount) = split_comparator(rest);
                let y = crate::svar::resolve_numeric_value(game, sa, amount, 0);
                compare(num_targets(sa) as i32, comparator, y)
            } else if let Some(defined) = property.strip_prefix("IsTargeting ") {
                is_targeting_defined(sa, &defined.replace('~', "+"), context)
            } else if property.starts_with("cmc") {
                ability_host.is_some_and(|host| {
                    let y = if host.zone == ZoneType::Stack || sa.is_spell {
                        spell_mana_value(sa, host)
                    } else {
                        sa.pay_costs
                            .as_ref()
                            .map_or(0, |cost| cost.get_total_mana().cmc())
                    };
                    crate::card::valid_filter::compare_raw_operand(y, &property[3..], Some(context))
                })
            } else if property.starts_with("NamedAbility") {
                false
            } else {
                ability_host.is_none_or(|host| {
                    crate::card::valid_filter::matches_valid_card_selector_with_context(
                        &crate::parsing::cached_compiled_selector(&format!("Card.{property}")),
                        host,
                        context,
                    )
                })
            }
        }
    }
}

fn is_mana_ability(sa: &SpellAbility, context: MatchContext<'_>) -> bool {
    if sa.is_spell || sa.uses_targeting() || sa.is_pw_ability() {
        return false;
    }
    if sa.is_trigger {
        let mana_trigger = sa.get_trigger(context.game).is_some_and(|trigger| {
            matches!(
                trigger.kind,
                TriggerType::TapsForMana | TriggerType::ManaAdded
            )
        });
        if !mana_trigger {
            return false;
        }
    }
    sa.is_mana_ability || matches!(sa.api, Some(ApiType::Mana | ApiType::ManaReflected))
}

/// Java moves a spell to the stack before any cost or trigger reads its mana value; the Rust
/// cast reads it from the zone it is cast from, so the paid X is added back here.
fn spell_mana_value(sa: &SpellAbility, host: &Card) -> i32 {
    let mut mana_value = host.mana_value();
    if host.zone != ZoneType::Stack && sa.source == Some(host.id) {
        mana_value += sa.x_mana_cost_paid as i32 * host.mana_cost.count_x() as i32;
    }
    mana_value
}

fn split_comparator(rest: &str) -> (&str, &str) {
    rest.split_at(2.min(rest.len()))
}

/// Mirrors Java's `Expressions.compare(int, String, int)`.
fn compare(left: i32, comparator: &str, right: i32) -> bool {
    if comparator.contains("LT") {
        left < right
    } else if comparator.contains("LE") {
        left <= right
    } else if comparator.contains("EQ") {
        left == right
    } else if comparator.contains("GE") {
        left >= right
    } else if comparator.contains("GT") {
        left > right
    } else if comparator.contains("NE") {
        left != right
    } else if comparator.contains("M2") {
        left % 2 == right % 2
    } else {
        false
    }
}

fn single_target(sa: &SpellAbility) -> bool {
    let mut num = 0;
    for targets in sa.get_all_target_choices() {
        num += targets.size();
        if num > 1 {
            return false;
        }
    }
    num == 1
}

fn num_targets(sa: &SpellAbility) -> usize {
    let mut cards: Vec<CardId> = Vec::new();
    let mut players: Vec<PlayerId> = Vec::new();
    let mut stack_entries: Vec<u32> = Vec::new();
    for targets in sa.get_all_target_choices() {
        for card in targets.all_target_cards() {
            if !cards.contains(&card) {
                cards.push(card);
            }
        }
        for player in targets.all_target_players() {
            if !players.contains(&player) {
                players.push(player);
            }
        }
        if let Some(entry) = targets.target_stack_entry {
            if !stack_entries.contains(&entry) {
                stack_entries.push(entry);
            }
        }
    }
    cards.len() + players.len() + stack_entries.len()
}

fn context_spell_ability<'a>(context: &MatchContext<'a>) -> std::borrow::Cow<'a, SpellAbility> {
    match context.spell_ability {
        Some(sa) => std::borrow::Cow::Borrowed(sa),
        None => std::borrow::Cow::Owned(SpellAbility::new_empty(
            Some(context.source_card.id),
            context.source_controller,
        )),
    }
}

fn defined_players(defined: &str, context: MatchContext<'_>) -> Vec<PlayerId> {
    crate::ability::ability_utils::resolve_defined_players_with_sa(
        defined,
        &context_spell_ability(&context),
        context.source_controller,
        context.game,
    )
}

fn is_targeting_defined(sa: &SpellAbility, defined: &str, context: MatchContext<'_>) -> bool {
    let players = defined_players(defined, context);
    let cards = crate::ability::ability_utils::get_defined_cards_for_sa(
        context.game,
        Some(context.source_card.id),
        defined,
        Some(context.source_controller),
        context.spell_ability,
    );
    let mut node = Some(sa);
    while let Some(current) = node {
        let targets = &current.target_chosen;
        if targets
            .all_target_cards()
            .iter()
            .any(|card| cards.contains(card))
            || targets
                .all_target_players()
                .iter()
                .any(|player| players.contains(player))
        {
            return true;
        }
        node = current.sub_ability.as_deref();
    }
    false
}

fn mana_from(
    from_what: &str,
    sa: &SpellAbility,
    ability_host: Option<&Card>,
    context: MatchContext<'_>,
) -> bool {
    let Some(host) = ability_host else {
        return false;
    };
    let game = context.game;
    let source = context.source_card;
    let (from_what, to_find) = match from_what.split_once('_') {
        Some((valid, amount)) => (
            valid,
            crate::svar::resolve_svar_expression(amount, game, source.id, source.controller, sa),
        ),
        None => (from_what, 1),
    };
    let selector = crate::parsing::cached_compiled_selector(from_what);
    let mut found = 0;
    for &mana_source in host.paying_sources_to_cast.iter().flatten() {
        if crate::card::valid_filter::matches_valid_card_selector_with_context(
            &selector,
            game.card(mana_source),
            context,
        ) {
            found += 1;
            if found == to_find {
                break;
            }
        }
    }
    found == to_find
}

fn could_cast_timing(
    sa: &SpellAbility,
    ability_host: Option<&Card>,
    context: MatchContext<'_>,
) -> bool {
    let Some(host) = ability_host else {
        return false;
    };
    let game = context.game;
    if crate::spellability::has_split_second_on_stack(game) {
        return false;
    }
    if crate::player::can_cast_sorcery(game, context.source_controller)
        || sa.restriction.variables.instant_speed()
    {
        return true;
    }
    if sa.is_spell {
        return host.type_line.is_instant()
            || host.has_keyword("Flash")
            || crate::staticability::static_ability_cast_with_flash::any_with_flash_for_card(
                game,
                host,
                context.source_controller,
            );
    }
    if sa.is_activated_ability() {
        return !sa.is_pw_ability() && !sa.restriction.variables.sorcery_speed();
    }
    true
}
