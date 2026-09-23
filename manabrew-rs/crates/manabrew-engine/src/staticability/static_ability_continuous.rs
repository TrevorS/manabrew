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
    if !st_ab.ir.may_play || !st_ab.check_conditions(source, game) {
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
    if st_ab
        .ir
        .may_play_limit
        .is_some_and(|limit| may_play_turn(st_ab, source, game) >= limit)
    {
        return false;
    }
    may_play_affects(st_ab, source, card, game)
}

fn may_play_affects(st_ab: &StaticAbility, source: &Card, card: &Card, game: &GameState) -> bool {
    if let Some(defined) = st_ab.ir.affected_defined.as_deref() {
        return crate::ability::ability_utils::get_defined_cards(
            game,
            Some(source.id),
            defined,
            Some(source.controller),
        )
        .contains(&card.id);
    }
    crate::card::valid_filter::matches_valid_card_selector_opt_in_game(
        st_ab.ir.affected.as_ref(),
        card,
        source,
        game,
    )
}

fn granted_statics(st_ab: &StaticAbility, source: &Card, game: &GameState) -> Vec<StaticAbility> {
    if !st_ab.check_conditions(source, game)
        || !crate::card::valid_filter::matches_valid_card_selector_opt(
            st_ab.ir.affected.as_ref(),
            source,
            source,
        )
    {
        return Vec::new();
    }
    let Some(add_static) = st_ab.ir.add_static_ability_text.as_deref() else {
        return Vec::new();
    };
    add_static
        .split(" & ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|svar_name| source.svars.get(svar_name))
        .filter_map(|static_text| crate::staticability::parse_static_ability(static_text))
        .collect()
}

pub fn can_play_or_granted(
    st_ab: &StaticAbility,
    source: &Card,
    card: &Card,
    game: &GameState,
) -> bool {
    can_play(st_ab, source, card, game)
        || granted_statics(st_ab, source, game)
            .iter()
            .any(|granted| can_play(granted, source, card, game))
}

/// `grants_zone_permissions` for one spell ability: Java's `checkZoneRestrictions`
/// (`SpellAbilityRestriction.java:250`) skips a grant whose `ValidSA$` the ability does not match.
pub fn grants_zone_permissions_for(
    st_ab: &StaticAbility,
    source: &Card,
    card: &Card,
    game: &GameState,
    sa: &crate::spellability::SpellAbility,
) -> bool {
    let accepts = |granting: &StaticAbility| {
        granting.ir.valid_sa.as_deref().is_none_or(|filter| {
            crate::spellability::matches_valid_sa(filter, sa, source, Some(card))
        })
    };
    if can_play(st_ab, source, card, game) {
        return st_ab.ir.may_play_grants_zone_permissions && accepts(st_ab);
    }
    granted_statics(st_ab, source, game)
        .iter()
        .any(|granted| can_play(granted, source, card, game) && accepts(granted))
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

/// Keep in sync with `StaticAbilityContinuous`: a `MayPlay$` grant is for the player
/// `MayPlayPlayer$` names, defined from the affected card, and otherwise for the static's controller.
pub fn may_play_player(
    st_ab: &StaticAbility,
    source: &Card,
    card: &Card,
    game: &GameState,
) -> crate::ids::PlayerId {
    match st_ab.ir.may_play_player.as_deref() {
        Some("CardOwner") => card.owner,
        Some("ActivePlayer" | "Player.Active") => game.active_player(),
        Some("Player") => game.player_order[0],
        _ => source.controller,
    }
}

/// The statics in any `ZoneType.STATIC_ABILITIES_SOURCE_ZONES` whose `MayPlay$` grant for `card`
/// would be `player`'s, in player order. `check_conditions` still gates each one on its host's
/// zone, so a static without `EffectZone$` keeps needing the battlefield.
pub fn may_play_grants<'a>(
    game: &'a GameState,
    player: crate::ids::PlayerId,
    card: &'a Card,
) -> impl Iterator<Item = (&'a Card, &'a StaticAbility)> + 'a {
    game.player_order
        .iter()
        .flat_map(move |&pid| {
            [
                forge_foundation::ZoneType::Battlefield,
                forge_foundation::ZoneType::Graveyard,
                forge_foundation::ZoneType::Exile,
                forge_foundation::ZoneType::Command,
                forge_foundation::ZoneType::Stack,
            ]
            .into_iter()
            .flat_map(move |zone| game.cards_in_zone(zone, pid).iter())
        })
        .flat_map(move |&source_id| {
            let source = game.card(source_id);
            source
                .static_abilities
                .iter()
                .map(move |st_ab| (source, st_ab))
        })
        .filter(move |(source, st_ab)| may_play_player(st_ab, source, card, game) == player)
}

/// Java `GameActionUtil:371` copies a grant's `ValidAfterStack$` onto the ability it builds for
/// that grant, and `SpellAbility.isLegalAfterStack` tests the spell against it. The restriction has
/// to be captured that way because by the time it is checked the card is on the stack, so neither
/// its zone nor an `Affected$` position like `TopLibrary` still holds. This port has one play
/// option rather than one per grant, so it keys off `cast_from` instead and asks whether some grant
/// opening that origin zone accepts the spell. With one such grant the two agree exactly.
pub fn may_play_allows_after_stack(
    game: &GameState,
    player: crate::ids::PlayerId,
    card: &Card,
    sa: &crate::spellability::SpellAbility,
) -> bool {
    let Some(origin) = card.cast_from else {
        return true;
    };
    if origin == forge_foundation::ZoneType::Hand {
        return true;
    }
    let mut grants = may_play_grants(game, player, card)
        .filter(|(source, st_ab)| {
            st_ab.ir.may_play
                && st_ab.ir.affected_zones.contains(&origin)
                && st_ab.check_conditions(source, game)
        })
        .peekable();
    if grants.peek().is_none() {
        return true;
    }
    grants.any(
        |(source, st_ab)| match st_ab.ir.valid_after_stack.as_deref() {
            Some(filter) => crate::spellability::matches_valid_sa(filter, sa, source, Some(card)),
            None => true,
        },
    )
}

/// Java `StaticAbility.getMayPlayTurn`: the lands played through the grant plus the spells cast
/// this turn whose `getMayPlay()` is this static.
pub fn may_play_turn(st_ab: &StaticAbility, source: &Card, game: &GameState) -> i32 {
    let index = static_index(st_ab, source);
    let spells = game
        .stack
        .get_spells_cast_this_turn()
        .iter()
        .filter(|&&cid| {
            game.card(cid).cast_sa.as_deref().is_some_and(|sa| {
                sa.may_play_source == Some(source.id) && sa.may_play_static == index
            })
        })
        .count() as i32;
    st_ab.may_play_turn + spells
}

fn static_index(st_ab: &StaticAbility, source: &Card) -> Option<usize> {
    source
        .static_abilities
        .iter()
        .position(|candidate| std::ptr::eq(candidate, st_ab))
}

/// The first `MayPlay$` grant that lets `player` play `card` from where it is, in the order
/// `getMayPlaySpellOptions` walks them, as its host and, for a static printed on the host, its
/// index. Java records the grant on the ability it builds; with one play option per card the first
/// covering grant is the one that built it.
pub fn may_play_grant_source(
    game: &GameState,
    player: crate::ids::PlayerId,
    card: &Card,
) -> Option<(crate::ids::CardId, Option<usize>)> {
    may_play_grants(game, player, card)
        .find(|(source, st_ab)| can_play_or_granted(st_ab, source, card, game))
        .map(|(source, st_ab)| (source.id, static_index(st_ab, source)))
}

/// Java `Card.mayPlay(player)` is not empty: a `MayPlay$` grant for `player` covers `card`.
pub fn player_may_play(game: &GameState, player: crate::ids::PlayerId, card: &Card) -> bool {
    may_play_grants(game, player, card)
        .any(|(source, st_ab)| can_play_or_granted(st_ab, source, card, game))
}

/// The `MayPlayAltManaCost$` of every grant that lets `player` cast `card`,
/// in the order Java's `getMayPlaySpellOptions` walks `Card.mayPlay`.
pub fn may_play_alt_costs(
    game: &GameState,
    player: crate::ids::PlayerId,
    card: &Card,
) -> Vec<String> {
    may_play_grants(game, player, card)
        .filter_map(|(source, st_ab)| may_play_alt_mana_cost(st_ab, source, card, game))
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
    if st_ab.ir.may_play_without_mana_cost {
        return Some("0".to_string());
    }
    st_ab.ir.may_play_alt_mana_cost.clone()
}

pub fn run(st_ab: &StaticAbility, source: &Card, game: &GameState) -> bool {
    st_ab.check_conditions(source, game)
}
