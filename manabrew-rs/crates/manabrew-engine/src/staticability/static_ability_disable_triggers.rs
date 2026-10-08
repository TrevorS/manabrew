use forge_foundation::ZoneType;

use crate::card::card_zone_table::CardZoneTable;
use crate::card::{valid_filter, Card};
use crate::event::RunParams;
use crate::game::GameState;
use crate::ids::CardId;
use crate::parsing::CompiledSelector;
use crate::spellability::SpellAbility;
use crate::trigger::Trigger;
use crate::trigger::TriggerType;

pub fn is_disabled(
    game: &GameState,
    trigger_host: CardId,
    regtrig: &Trigger,
    overriding_ability: &dyn Fn() -> SpellAbility,
    run_params: &RunParams,
) -> bool {
    let host = game.card(trigger_host);
    let mut cards_filtered = None;
    for source in game
        .cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
    {
        for st_ab in source.static_abilities.iter().filter(|sa| {
            sa.check_conditions_full(
                &crate::staticability::StaticMode::DisableTriggers,
                source,
                game,
            )
        }) {
            if let Some(valid_mode) = st_ab.ir.valid_mode.as_deref() {
                let modes = valid_mode.split(',').map(|s| s.trim());
                let trig_mode = regtrig.kind.name();
                if !modes.clone().any(|m| m.eq_ignore_ascii_case(trig_mode)) {
                    continue;
                }
            }
            if st_ab.ir.valid_card.is_some() && regtrig.get_spawning_ability().is_some() {
                continue;
            }
            if let Some(valid_card) = st_ab.ir.valid_card.as_ref() {
                if !matches_valid_card(valid_card, host, source, game) {
                    continue;
                }
            }
            if let Some(valid_trigger) = st_ab.ir.valid_trigger.as_deref() {
                if !matches_valid_trigger(
                    valid_trigger,
                    game,
                    source,
                    trigger_host,
                    &overriding_ability(),
                ) {
                    continue;
                }
            }
            if regtrig.kind == TriggerType::ChangesZoneAll {
                if !changes_zone_all_disabled(
                    st_ab,
                    game,
                    regtrig,
                    run_params,
                    source,
                    &mut cards_filtered,
                ) {
                    continue;
                }
            } else if !mode_specific_matches(st_ab, game, regtrig, run_params, source) {
                continue;
            }
            return true;
        }
    }
    false
}

/// Keep in sync with the source and static filters of `is_disabled`.
pub fn has_disable_triggers_ability(game: &GameState) -> bool {
    game.cards
        .iter()
        .filter(|c| c.zone.is_static_ability_source())
        .any(|source| {
            source.static_abilities.iter().any(|sa| {
                sa.check_conditions_full(
                    &crate::staticability::StaticMode::DisableTriggers,
                    source,
                    game,
                )
            })
        })
}

fn mode_specific_matches(
    st_ab: &crate::staticability::StaticAbility,
    game: &GameState,
    regtrig: &Trigger,
    run_params: &RunParams,
    source: &Card,
) -> bool {
    let source_controller = source.controller;
    match regtrig.kind {
        TriggerType::ChangesZone => {
            let origin = regtrig.origin_zone();
            let moved = if origin == Some(ZoneType::Battlefield) {
                run_params.card_lki
            } else {
                run_params.card
            };
            if let Some(valid_cause) = st_ab.ir.valid_cause.as_ref() {
                let Some(cid) = moved else {
                    return false;
                };
                if !matches_valid_card(valid_cause, game.card(cid), source, game) {
                    return false;
                }
            }
            if !st_ab.ir.origin_zones.is_empty()
                && !matches_zones(&st_ab.ir.origin_zones, run_params.origin)
            {
                return false;
            }
            if !st_ab.ir.destination_zones.is_empty()
                && !matches_zones(&st_ab.ir.destination_zones, run_params.destination)
            {
                return false;
            }
            true
        }
        TriggerType::SpellCast
        | TriggerType::AbilityCast
        | TriggerType::SpellAbilityCast
        | TriggerType::SpellCastOrCopy
        | TriggerType::SpellCopied
        | TriggerType::SpellCopy
        | TriggerType::SpellAbilityCopy => {
            if let Some(valid_cause) = st_ab.ir.valid_cause.as_ref() {
                let Some(cid) = run_params.spell_card else {
                    return false;
                };
                if !matches_valid_card(valid_cause, game.card(cid), source, game) {
                    return false;
                }
            }
            if let Some(valid_activator) = st_ab.ir.valid_activator.as_ref() {
                let Some(pid) = run_params.spell_controller else {
                    return false;
                };
                if !valid_filter::matches_valid_player_selector_in_game(
                    valid_activator,
                    pid,
                    source,
                    source_controller,
                    game,
                ) {
                    return false;
                }
            }
            true
        }
        TriggerType::Attacks => {
            if let Some(valid_cause) = st_ab.ir.valid_cause.as_ref() {
                let Some(attacker) = run_params.attacker else {
                    return false;
                };
                if !matches_valid_card(valid_cause, game.card(attacker), source, game) {
                    return false;
                }
            }
            true
        }
        TriggerType::DamageDone | TriggerType::DamageDealtOnce => {
            if let Some(wanted) = st_ab.ir.combat_damage {
                if run_params.is_combat_damage != Some(wanted) {
                    return false;
                }
            }
            if let Some(valid_source) = st_ab.ir.valid_source.as_ref() {
                let Some(source_id) = run_params.damage_source else {
                    return false;
                };
                if !matches_valid_card(valid_source, game.card(source_id), source, game) {
                    return false;
                }
            }
            if let Some(valid_target) = st_ab.ir.valid_target.as_ref() {
                if let Some(target_card) = run_params.damage_target_card {
                    if !matches_valid_card(valid_target, game.card(target_card), source, game) {
                        return false;
                    }
                } else if let Some(target_player) = run_params.damage_target_player {
                    if !valid_filter::matches_valid_player_selector_in_game(
                        valid_target,
                        target_player,
                        source,
                        source_controller,
                        game,
                    ) {
                        return false;
                    }
                }
            }
            true
        }
        _ => true,
    }
}

fn changes_zone_all_disabled(
    st_ab: &crate::staticability::StaticAbility,
    game: &GameState,
    regtrig: &Trigger,
    run_params: &RunParams,
    source: &Card,
    cards_filtered: &mut Option<CardZoneTable>,
) -> bool {
    let Some(table) = cards_filtered
        .as_ref()
        .or(run_params.change_zone_table.as_ref())
    else {
        return false;
    };
    let mut filtered = table.with_same_last_state();
    let mut possibly_disabled = false;
    for (origin, destination, cards) in table.cells() {
        let mut changers = cards.to_vec();
        if (st_ab.ir.origin_zones.is_empty() || st_ab.ir.origin_zones.contains(&origin))
            && (st_ab.ir.destination_zones.is_empty()
                || st_ab.ir.destination_zones.contains(&destination))
        {
            changers.retain(|&card_id| {
                let lki = (origin == ZoneType::Battlefield)
                    .then(|| crate::lki::battlefield_lki_card(game, card_id))
                    .flatten();
                let card = lki.as_ref().unwrap_or_else(|| game.card(card_id));
                !st_ab
                    .ir
                    .valid_cause
                    .as_ref()
                    .is_none_or(|valid_cause| matches_valid_card(valid_cause, card, source, game))
            });
            if changers.len() < cards.len() {
                possibly_disabled = true;
            }
        }
        filtered.put_cell(origin, destination, changers);
    }
    if !possibly_disabled {
        return false;
    }
    let run_params_filtered = RunParams {
        cards: Some(filtered.all_cards()),
        zone_changes: Some(filtered.zone_changes()),
        change_zone_table: Some(filtered.clone()),
        ..run_params.clone()
    };
    if regtrig
        .mode
        .perform_test(regtrig, &run_params_filtered, game)
    {
        *cards_filtered = Some(filtered);
        return false;
    }
    true
}

pub(crate) fn matches_valid_trigger(
    valid_trigger: &str,
    game: &GameState,
    source: &Card,
    trigger_host: CardId,
    trigger_sa: &SpellAbility,
) -> bool {
    crate::spellability::matches_valid_sa(
        valid_trigger,
        trigger_sa,
        Some(game.card(trigger_host)),
        valid_filter::MatchContext::new(source, game),
    )
}

pub(crate) fn matches_valid_card(
    valid: &CompiledSelector,
    card: &Card,
    source: &Card,
    game: &GameState,
) -> bool {
    valid_filter::matches_valid_card_selector_in_game(valid, card, source, game)
}

#[allow(dead_code)]
fn matches_zone(filter: &str, zone: Option<ZoneType>) -> bool {
    let Some(zone) = zone else {
        return false;
    };
    match filter.to_ascii_lowercase().as_str() {
        "battlefield" => zone == ZoneType::Battlefield,
        "hand" => zone == ZoneType::Hand,
        "graveyard" => zone == ZoneType::Graveyard,
        "library" => zone == ZoneType::Library,
        "exile" => zone == ZoneType::Exile,
        "stack" => zone == ZoneType::Stack,
        "command" => zone == ZoneType::Command,
        "any" => true,
        _ => true,
    }
}

fn matches_zones(filters: &[ZoneType], zone: Option<ZoneType>) -> bool {
    let Some(zone) = zone else {
        return false;
    };
    filters.contains(&zone)
}
