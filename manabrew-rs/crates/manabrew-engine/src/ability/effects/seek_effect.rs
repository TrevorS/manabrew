//! Seek effect — randomly find cards matching criteria from library.
//!
//! Ported 1:1 from Java's `SeekEffect.java`.
//! Seek N [Type]: Randomly select N cards matching [Type] from your library
//! and put them into your hand. (Arena digital mechanic — no player choice.)

use forge_foundation::ZoneType;

use super::{emit_zone_trigger, EffectContext};
use crate::card::card_zone_table::CardZoneTable;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::parsing::keys;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `SeekEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(SeekEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let seek_types: Vec<String> = match crate::parsing::raw_get(&sa.ability_text, "Types") {
        Some(types) => types.split(',').map(|t| t.trim().to_string()).collect(),
        None => vec![crate::parsing::raw_get(&sa.ability_text, "Type")
            .unwrap_or("Card")
            .trim()
            .to_string()],
    };

    let seek_num = super::resolve_numeric_svar(ctx.game, sa, "Num", 1);
    if seek_num <= 0 {
        return;
    }

    let mut trigger_list = CardZoneTable::default();
    let players = crate::ability::spell_ability_effect::get_target_players(ctx.game, sa);

    for seeker in players {
        if crate::player::has_lost(ctx.game, seeker) {
            continue;
        }
        let mut sought: Vec<CardId> = Vec::new();

        for seek_type in &seek_types {
            let mut pool: Vec<CardId> =
                match crate::parsing::raw_get(&sa.ability_text, keys::DEFINED_CARDS) {
                    Some(defined) => crate::ability::ability_utils::get_defined_cards_for_sa(
                        ctx.game,
                        sa.source,
                        defined,
                        Some(sa.activating_player),
                        Some(sa),
                    ),
                    None => ctx
                        .game
                        .cards_in_zone(ZoneType::Library, seeker)
                        .iter()
                        .rev()
                        .copied()
                        .collect(),
                };
            if seek_type != "Card" {
                pool.retain(|&cid| {
                    crate::ability::ability_utils::matches_valid_cards_for_sa(
                        ctx.game,
                        sa,
                        ctx.game.card(cid),
                        None,
                        seek_type,
                    )
                });
            }
            if pool.is_empty() {
                continue;
            }

            for card_id in crate::util::aggregates::random(&pool, seek_num as usize, ctx.rng) {
                let old_zone = ctx.game.card(card_id).zone;
                ctx.move_card(card_id, ZoneType::Hand, ctx.game.card(card_id).owner);
                let result_zone = ctx.game.card(card_id).zone;
                if result_zone != ZoneType::Library {
                    trigger_list.put(Some(ZoneType::Library), Some(result_zone), card_id);
                    emit_zone_trigger(ctx.trigger_handler, card_id, old_zone, result_zone);
                }
                if result_zone == ZoneType::Hand {
                    sought.push(card_id);
                }
            }
        }

        if !sought.is_empty() {
            if let Some(sid) = sa.source {
                if sa.ir.remember_found {
                    for &cid in &sought {
                        ctx.game.host_object_mut(sid, sa).add_remembered_card(cid);
                    }
                }
                if sa.ir.imprint_found {
                    for &cid in &sought {
                        ctx.game.host_object_mut(sid, sa).add_imprinted_card(cid);
                    }
                }
            }
            ctx.trigger_handler.run_trigger(
                TriggerType::SeekAll,
                RunParams {
                    player: Some(seeker),
                    cards: Some(sought),
                    ..Default::default()
                },
                false,
            );
        }
    }
    if !trigger_list.all_cards().is_empty() {
        trigger_list.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
    }
}
