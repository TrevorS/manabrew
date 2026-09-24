//! ChangeZone effect — moves cards between zones.
//!
//! Ported from Java's `ChangeZoneEffect.java`. Sub-modules:
//! - [`helpers`] — matching, pre/post move, destination resolution, search restrictions
//! - [`known`] — known-origin resolve (Battlefield, Graveyard, targeted cards)
//! - [`hidden`] — hidden-origin resolve (Library/Hand searches)
//! - [`search`] — search sub-routines (single, multi, each, random, player choice)
//! - [`stack`] — stack removal (bouncing/exiling spells)
//! - [`move_cards`] — shared move + post-processing logic

pub(super) mod helpers;
mod hidden;
mod known;
pub(super) mod move_cards;
pub(super) mod search;
mod stack;

use forge_foundation::ZoneType;

use super::EffectContext;
use crate::spellability::SpellAbility;

/// Struct form so this directory-module effect can participate in the
/// `SpellAbilityEffect` trait hierarchy alongside the single-file effects.
#[manabrew_engine_macros::spell_effect(ChangeZoneEffect)]
fn resolve(ctx: &mut EffectContext, sa: &SpellAbility) {
    if !crate::ability::spell_ability_effect::check_valid_duration(
        ctx.game,
        sa,
        sa.ir.duration.as_ref(),
    ) {
        return;
    }
    resolve(ctx, sa);
}

/// Configure the spell ability during construction.
/// Mirrors Java `ChangeZoneEffect.buildSpellAbility` — calls
/// `adjustChangeZoneTarget` to set the target zone to the origin zone.
pub fn build_spell_ability(sa: &mut SpellAbility) {
    let origins = sa.origin_zones();
    if !origins.is_empty() {
        if let Some(ref mut tr) = sa.target_restrictions {
            if !tr.can_tgt_player() {
                tr.tgt_zone = origins;
            }
        }
    }
}

/// Top-level resolve dispatcher — mirrors Java's `resolve()` which splits on
/// `sa.isHidden()` into hidden-origin (library search) vs known-origin paths.
pub fn resolve(ctx: &mut EffectContext, sa: &SpellAbility) {
    let destination = sa.destination_zone();

    // Multi-origin: Origin$ can be comma-separated (e.g. "Library,Graveyard")
    let origins: Vec<ZoneType> = sa.origin_zones();

    if origins.is_empty() {
        let Some(dest_zone) = destination else {
            return;
        };
        // Java's `Origin$ All` includes the library, so `changeHiddenOriginResolve` shuffles the
        // fetcher's library before a move to it and again for `Shuffle$ True`.
        let is_origin_all = sa.origin().is_some_and(|o| o.eq_ignore_ascii_case("All"));
        if !is_origin_all && sa.origin().is_some() {
            return;
        }
        let defined_cards = if is_origin_all {
            crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa)
        } else {
            crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa)
        };
        if defined_cards.is_empty() {
            return;
        }
        let mut zones: Vec<ZoneType> = Vec::new();
        for cid in &defined_cards {
            let zone = ctx.game.card(*cid).zone;
            if zone != ZoneType::None && !zones.contains(&zone) {
                zones.push(zone);
            }
        }
        let fetchers = if is_origin_all {
            search::resolve_defined_players_for_hidden_origin(ctx, sa)
        } else {
            Vec::new()
        };
        let shuffle_mandatory = !sa.ir.no_shuffle && sa.ir.shuffle_raw.as_deref() != Some("False");
        if dest_zone == ZoneType::Library && shuffle_mandatory {
            for &pid in &fetchers {
                shuffle_library(ctx, pid);
            }
        }
        let mut sa_no_shuffle = sa.clone();
        let ir = std::sync::Arc::make_mut(&mut sa_no_shuffle.ir);
        ir.no_shuffle = true;
        if is_origin_all {
            ir.shuffle = false;
        }
        for zone in zones {
            known::resolve_known_origin(ctx, &sa_no_shuffle, zone, dest_zone);
        }
        if sa.is_shuffle() {
            for &pid in &fetchers {
                shuffle_library(ctx, pid);
            }
        }
        return;
    }

    let primary_origin = origins[0];

    // Java parity: sa.isHidden() && !sa.isNinjutsu() → hidden path
    if (primary_origin.is_hidden() || sa.is_hidden()) && !sa.ir.ninjutsu {
        hidden::resolve_hidden_origin(ctx, sa, primary_origin, destination);
    } else if let Some(dest_zone) = destination {
        let mut known_origins: Vec<ZoneType> = Vec::new();
        if sa.uses_targeting() && origins.len() > 1 && !origins.contains(&ZoneType::Stack) {
            for cid in sa.target_chosen.all_target_cards() {
                let zone = ctx.game.card(cid).zone;
                if origins.contains(&zone) && !known_origins.contains(&zone) {
                    known_origins.push(zone);
                }
            }
        }
        if known_origins.is_empty() {
            known_origins.push(primary_origin);
        }
        for zone in known_origins {
            known::resolve_known_origin(ctx, sa, zone, dest_zone);
        }
    }
}

fn shuffle_library(ctx: &mut EffectContext, pid: crate::ids::PlayerId) {
    ctx.game.shuffle_zone_cards(ZoneType::Library, pid, ctx.rng);
    ctx.trigger_handler.run_trigger(
        crate::trigger::TriggerType::Shuffled,
        crate::event::RunParams {
            player: Some(pid),
            ..Default::default()
        },
        false,
    );
}
