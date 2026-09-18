//! Java parity bridge for `StaticAbilityContinuous.java`.
//! Canonical CR613 recomputation lives in `layer.rs`.

use crate::card::Card;
use crate::game::GameState;
use crate::staticability::{Layer, StaticAbility};

pub fn apply_continuous_ability(
    st_ab: &StaticAbility,
    source: &Card,
    game: &mut GameState,
    layer: Layer,
) {
    if !crate::staticability::layer::classify_static_layers(st_ab).contains(&layer) {
        return;
    }
    if !st_ab.check_conditions(source, game) {
        return;
    }
    crate::staticability::layer::apply_continuous_effects(game);
}

pub fn resolve(st_ab: &StaticAbility, source: &Card, game: &GameState) {
    let _ = run(st_ab, source, game);
}

pub fn can_play(st_ab: &StaticAbility, source: &Card, card: &Card, game: &GameState) -> bool {
    if !st_ab.ir.may_play {
        return false;
    }
    // Check AffectedZone$ — the zone where the affected cards must be.
    // Default is Hand if not specified (normal MayPlay like casting from hand).
    if !st_ab.ir.affected_zones.is_empty() {
        if !st_ab.ir.affected_zones.contains(&card.zone) {
            return false;
        }
    } else if card.zone != forge_foundation::ZoneType::Hand {
        return false;
    }
    if let Some(defined) = st_ab.ir.affected_defined.as_deref() {
        return crate::ability::ability_utils::get_defined_cards(
            game,
            Some(source.id),
            defined,
            Some(source.controller),
        )
        .contains(&card.id);
    }
    crate::card::valid_filter::matches_valid_card_selector_opt(
        st_ab.ir.affected.as_ref(),
        card,
        source,
    )
}

pub fn can_play_or_granted(
    st_ab: &StaticAbility,
    source: &Card,
    card: &Card,
    game: &GameState,
) -> bool {
    if can_play(st_ab, source, card, game) {
        return true;
    }
    if !st_ab.check_conditions(source, game) {
        return false;
    }
    if !crate::card::valid_filter::matches_valid_card_selector_opt(
        st_ab.ir.affected.as_ref(),
        source,
        source,
    ) {
        return false;
    }
    let Some(add_static) = st_ab.ir.add_static_ability_text.as_deref() else {
        return false;
    };
    add_static
        .split(" & ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|svar_name| source.svars.get(svar_name))
        .filter_map(|static_text| crate::staticability::parse_static_ability(static_text))
        .any(|granted| can_play(&granted, source, card, game))
}

/// Java `SpellAbilityRestriction.checkZoneRestrictions`: a grant made with
/// `MayPlayDontGrantZonePermissions$` lets its owner cast the card from hand,
/// and from another zone only when some other grant opens that zone.
pub fn grants_zone_permissions(
    st_ab: &StaticAbility,
    source: &Card,
    card: &Card,
    game: &GameState,
) -> bool {
    if can_play(st_ab, source, card, game) {
        return st_ab.ir.may_play_grants_zone_permissions;
    }
    can_play_or_granted(st_ab, source, card, game)
}

/// The `MayPlayAltManaCost$` of every grant that lets `player` cast `card`,
/// in the order Java's `getMayPlaySpellOptions` walks `Card.mayPlay`.
pub fn may_play_alt_costs(
    game: &GameState,
    player: crate::ids::PlayerId,
    card: &Card,
) -> Vec<String> {
    game.cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
        .iter()
        .chain(
            game.cards_in_zone(forge_foundation::ZoneType::Command, player)
                .iter(),
        )
        .flat_map(|&source_id| {
            let source = game.card(source_id);
            source
                .static_abilities
                .iter()
                .filter_map(move |st_ab| may_play_alt_mana_cost(st_ab, source, card, game))
        })
        .collect()
}

/// If a `MayPlay$ True` static on `source` grants `card` permission to be
/// cast and also defines `MayPlayAltManaCost$`, return that alt cost string.
/// Mirrors Java's `GameActionUtil.canPlayCardMayPlay` reading
/// `getMayPlayAltManaCost()` for the granting static.
pub fn may_play_alt_mana_cost(
    st_ab: &StaticAbility,
    source: &Card,
    card: &Card,
    game: &GameState,
) -> Option<String> {
    if !can_play(st_ab, source, card, game) {
        return None;
    }
    st_ab.ir.may_play_alt_mana_cost.clone()
}

pub fn run(st_ab: &StaticAbility, source: &Card, game: &GameState) -> bool {
    st_ab.check_conditions(source, game)
}
