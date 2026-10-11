use forge_foundation::ZoneType;

use super::{emit_zone_trigger, EffectContext};
use crate::ids::{CardId, PlayerId};

/// Configure the spell ability during construction.
/// Mirrors Java `ChangeZoneAllEffect.buildSpellAbility` — calls
/// `adjustChangeZoneTarget` to set the target zone to the origin zone.
pub fn build_spell_ability(sa: &mut crate::spellability::SpellAbility) {
    // If the SA has an Origin$ parameter and uses targeting, set the
    // target restriction zone to the origin zone so that targeting
    // looks in the correct zone (not just Battlefield).
    if let Some(zone) = sa.origin_zone() {
        if let Some(ref mut tr) = sa.target_restrictions {
            if !tr.can_tgt_player() {
                tr.tgt_zone = vec![zone];
            }
        }
    }
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ChangeZoneAllEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ChangeZoneAllEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    if !crate::ability::spell_ability_effect::check_valid_duration(
        ctx.game,
        sa,
        sa.ir.duration.as_ref(),
    ) {
        return;
    }
    let origin_zones = if sa.origin().is_none() {
        vec![ZoneType::Battlefield]
    } else {
        sa.origin_zones()
    };
    if origin_zones.is_empty() {
        return;
    }
    let Some(dest_zone) = sa
        .destination_zone()
        .or_else(|| sa.destination().is_none().then_some(ZoneType::Graveyard))
    else {
        return;
    };
    // Forge uses ChangeType$ as the primary filter for ChangeZoneAll; fall back to ValidCards$.
    let valid_cards_filter = sa
        .change_type()
        .or(sa.ir.valid_cards_text.as_deref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Card".to_string());
    let tapped = sa.ir.tapped;

    // For Duration$ UntilHostLeavesPlay, track the source card so we can return
    // exiled permanents when the source leaves the battlefield (e.g. Deputy of Detention).
    let until_host_leaves = sa.ir.duration.as_ref().is_some_and(|duration| {
        matches!(
            duration,
            crate::spellability::AbilityDuration::UntilHostLeavesPlay
                | crate::spellability::AbilityDuration::UntilHostLeavesPlayOrEot
        )
    });
    let exile_source = if until_host_leaves { sa.source } else { None };

    {
        let use_all_origin_zones =
            crate::parsing::raw_has_key(&sa.ability_text, "UseAllOriginZones");
        let player_ids: Vec<PlayerId> = if use_all_origin_zones {
            ctx.game.player_order.clone()
        } else if sa.uses_targeting() {
            sa.target_chosen.all_target_players()
        } else if let Some(defined) = sa.ir.defined_text.as_deref() {
            crate::ability::ability_utils::resolve_defined_players_with_sa(
                defined,
                sa,
                sa.activating_player,
                ctx.game,
            )
        } else {
            ctx.game.player_order.clone()
        };
        let mut to_move: Vec<(CardId, PlayerId)> = Vec::new();

        for &pid in &player_ids {
            let zone_cards: Vec<CardId> = origin_zones
                .iter()
                .flat_map(|&zone| {
                    let cards = ctx.game.cards_in_zone(zone, pid);
                    if zone == ZoneType::Library {
                        cards.iter().rev().copied().collect::<Vec<_>>()
                    } else {
                        cards.to_vec()
                    }
                })
                .collect();
            let by_type = crate::ability::ability_utils::filter_list_by_type(
                ctx.game,
                &zone_cards,
                &valid_cards_filter,
                sa,
            );
            for cid in zone_cards {
                if by_type.contains(&cid) {
                    let dest_owner = if dest_zone != ZoneType::Battlefield {
                        ctx.game.card(cid).owner
                    } else if sa.is_gain_control() {
                        sa.activating_player
                    } else {
                        ctx.game.card(cid).controller
                    };
                    to_move.push((cid, dest_owner));
                }
            }
        }

        if sa.ir.forget_other_remembered {
            if let Some(sid) = sa.source {
                ctx.game.host_object_mut(sid, sa).clear_remembered();
            }
        }

        if dest_zone == ZoneType::Library && to_move.len() > 1 && sa.ir.random_order {
            let mut cards = to_move.iter().map(|(cid, _)| *cid).collect::<Vec<_>>();
            ctx.rng.shuffle_cards(&mut cards);
            let mut ordered = Vec::with_capacity(to_move.len());
            for cid in cards {
                if let Some((_, owner)) = to_move.iter().find(|(card_id, _)| *card_id == cid) {
                    ordered.push((cid, *owner));
                }
            }
            to_move = ordered;
        }

        if !sa.ir.random_order && !sa.ir.shuffle {
            let cards: Vec<CardId> = to_move.iter().map(|(cid, _)| *cid).collect();
            let ordered = if dest_zone == ZoneType::Library && cards.len() >= 2 {
                let orderer = sa
                    .ir
                    .defined_player_text
                    .as_deref()
                    .and_then(|defined| {
                        crate::ability::ability_utils::resolve_defined_players_with_sa(
                            defined,
                            sa,
                            sa.activating_player,
                            ctx.game,
                        )
                        .first()
                        .copied()
                    })
                    .unwrap_or(sa.activating_player);
                ctx.agents[orderer.index()]
                    .order_move_to_zone_list(ctx.game, orderer, &cards, dest_zone)
            } else {
                ctx.game.order_cards_by_their_owners_for_sa(
                    cards,
                    dest_zone,
                    Some(sa),
                    &mut Some(&mut *ctx.agents),
                )
            };
            let mut reordered = Vec::with_capacity(to_move.len());
            for cid in ordered {
                if let Some(&entry) = to_move.iter().find(|(card_id, _)| *card_id == cid) {
                    reordered.push(entry);
                }
            }
            if reordered.len() == to_move.len() {
                to_move = reordered;
            }
        }

        let mut moved_to_library: Vec<(CardId, PlayerId)> = Vec::new();
        for (card_id, dest_owner) in to_move {
            if !origin_zones.contains(&ctx.game.card(card_id).zone) {
                continue; // already moved
            }
            let old_zone = ctx.game.card(card_id).zone;
            ctx.game.setup_static_effect(card_id, sa);
            if dest_zone == ZoneType::Battlefield && sa.is_face_down() {
                crate::card::card_factory_util::turn_face_down_with_state(
                    ctx.game.card_mut(card_id),
                );
                crate::card::card_factory_util::set_face_down_state(ctx.game, card_id, sa);
            }
            if dest_zone == ZoneType::Battlefield && sa.is_gain_control() {
                ctx.game
                    .card_mut(card_id)
                    .set_controller(sa.activating_player);
            }
            if dest_zone == ZoneType::Exile {
                ctx.exile(card_id, Some(sa));
            } else {
                ctx.move_card(card_id, dest_zone, dest_owner);
            }
            if sa.is_remember_changed() && ctx.game.card(card_id).zone != old_zone {
                let remembers = match crate::parsing::raw_get(
                    &sa.ability_text,
                    crate::parsing::keys::REMEMBER_CHANGED,
                ) {
                    Some(filter) if !filter.eq_ignore_ascii_case("True") => {
                        sa.source.is_some_and(|sid| {
                            crate::card::valid_filter::matches_valid_card_selector_with_context(
                                &crate::parsing::cached_compiled_selector(filter),
                                ctx.game.card(card_id),
                                crate::card::valid_filter::MatchContext::new(
                                    ctx.game.card(sid),
                                    ctx.game,
                                )
                                .with_host_object(sa),
                            )
                        })
                    }
                    _ => true,
                };
                if remembers {
                    if let Some(sid) = sa.source {
                        ctx.game
                            .host_object_mut(sid, sa)
                            .add_remembered_card(card_id);
                    }
                }
            }
            if dest_zone == ZoneType::Library {
                moved_to_library.push((card_id, dest_owner));
            }
            // Mark cards exiled by a UntilHostLeavesPlay effect so they can return
            // when the source leaves the battlefield.
            if dest_zone == ZoneType::Exile {
                if let Some(src_id) = exile_source {
                    ctx.game.card_mut(card_id).set_exiled_by(Some(src_id));
                    ctx.game.card_mut(card_id).until_host_leaves_origin = Some(old_zone);
                    ctx.game
                        .record_until_leaves_battlefield(card_id, src_id, old_zone, sa);
                }
                if let Some(sid) = sa.source.filter(|&sid| {
                    !ctx.game.card(card_id).is_token
                        && matches!(
                            ctx.game.card(sid).zone,
                            ZoneType::Battlefield | ZoneType::Stack | ZoneType::Command
                        )
                }) {
                    ctx.game.card_mut(sid).add_exiled_card(card_id);
                }
                crate::ability::spell_ability_effect::handle_exiled_with(ctx.game, sa, card_id);
            }
            if dest_zone == ZoneType::Battlefield {
                if tapped {
                    ctx.game.tap(card_id);
                }
                if let Some(counter_type) = sa.ir.with_counters_type.as_ref() {
                    if ctx.game.card(card_id).zone == ZoneType::Battlefield {
                        let amount = crate::svar::resolve_numeric_svar(
                            ctx.game,
                            sa,
                            crate::parsing::keys::WITH_COUNTERS_AMOUNT,
                            1,
                        );
                        ctx.add_counter(
                            card_id,
                            counter_type,
                            amount,
                            sa,
                            crate::event::RunParams {
                                source_player: Some(sa.activating_player),
                                ..Default::default()
                            },
                        );
                    }
                }
                ctx.trigger_handler
                    .register_active_trigger(ctx.game, card_id);
            }
            emit_zone_trigger(ctx.trigger_handler, card_id, old_zone, dest_zone);
        }

        if dest_zone == ZoneType::Library && sa.library_position() == Some("-1") {
            for &pid in &player_ids {
                let cards = moved_to_library
                    .iter()
                    .filter_map(|(cid, owner)| (*owner == pid).then_some(*cid))
                    .collect::<Vec<_>>();
                if !cards.is_empty() {
                    ctx.game
                        .move_cards_to_zone_bottom(ZoneType::Library, pid, &cards);
                }
            }
        }

        // Handle Shuffle$ (e.g. Nihil Spellbomb shuffles the target player's library).
        if sa.ir.shuffle_raw.is_some() {
            for &pid in &player_ids {
                ctx.game.shuffle_zone_cards(ZoneType::Library, pid, ctx.rng);
            }
        }
    }
}
