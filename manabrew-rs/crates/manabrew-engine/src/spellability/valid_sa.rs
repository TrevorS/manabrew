//! Mirrors Java's `SpellAbility.isValid` and `SpellAbility.hasProperty`.

use crate::card::valid_filter::MatchContext;
use crate::card::Card;
use crate::spellability::SpellAbility;

pub fn matches_valid_sa(
    filter: &str,
    sa: &SpellAbility,
    ability_host: Option<&Card>,
    context: MatchContext<'_>,
) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }
    filter
        .split(',')
        .map(str::trim)
        .filter(|restriction| !restriction.is_empty())
        .any(|restriction| is_valid(restriction, sa, ability_host, context))
}

pub fn is_valid(
    restriction: &str,
    sa: &SpellAbility,
    ability_host: Option<&Card>,
    context: MatchContext<'_>,
) -> bool {
    let (head, properties) = match restriction.split_once('.') {
        Some((head, properties)) => (head, Some(properties)),
        None => (restriction, None),
    };
    let (test_failed, head) = match head.strip_prefix('!') {
        Some(head) => (true, head),
        None => (false, head),
    };

    let head_matches = match head {
        "Spell" => sa.is_spell,
        "Ability" => !sa.is_spell,
        "Instant" => ability_host.is_some_and(|host| sa.card_state_type_line(host).is_instant()),
        "Sorcery" => ability_host.is_some_and(|host| sa.card_state_type_line(host).is_sorcery()),
        "Triggered" => sa.is_trigger,
        "Activated" => sa.is_activated_ability(),
        "Static" => sa.is_ability_static(),
        "SpellAbility" => true,
        head if head.contains("LandAbility") => sa.is_land_ability,
        _ => false,
    };
    if !head_matches {
        return test_failed;
    }

    if let Some(properties) = properties {
        for property in properties.split('+') {
            if !has_property(property, sa, ability_host, context) {
                return test_failed;
            }
        }
    }
    !test_failed
}

fn has_property(
    property: &str,
    sa: &SpellAbility,
    ability_host: Option<&Card>,
    context: MatchContext<'_>,
) -> bool {
    match property.strip_prefix('!') {
        Some(property) => {
            !super::spell_ability_property::has_property(sa, property, ability_host, context)
        }
        None => super::spell_ability_property::has_property(sa, property, ability_host, context),
    }
}
