use forge_foundation::ZoneType;

use super::{emit_zone_trigger, matches_change_type, resolve_numeric_svar, EffectContext};
use crate::card::valid_filter;
use crate::parsing::keys;

/// `SP$ DigUntil` — reveal cards from the top of library until finding N matching cards.
///
/// Mirrors Java's `DigUntilEffect.java`.
/// - `Amount$` — how many matching cards to find (default 1).
/// - `Valid$` — filter for matching cards (e.g. "Land", "Creature").
/// - `FoundDestination$` — where found cards go (default Hand).
/// - `RevealedDestination$` — where non-matching cards go (default Library).
/// - `RevealedLibraryPosition$` — top (0, the default) or bottom (negative).
///
/// # Card script examples
/// ```text
/// A:SP$ DigUntil | Valid$ Land | FoundDestination$ Hand | RevealedDestination$ Graveyard
/// A:SP$ DigUntil | Valid$ Creature | Amount$ 2 | FoundDestination$ Battlefield
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DigUntilEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DigUntilEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let amount = resolve_numeric_svar(ctx.game, sa, keys::AMOUNT, 1).max(0) as usize;

    let valid_selector = sa.ir.valid_filter_selector.as_ref();
    let valid_filter = sa.ir.valid_filter_text.as_deref().unwrap_or("Card");

    let found_dest = sa.ir.found_destination_zone;
    let revealed_dest = sa.ir.revealed_destination_zone.unwrap_or(ZoneType::Library);

    for target_player in crate::ability::spell_ability_effect::get_target_players(ctx.game, sa) {
        let lib_len = ctx
            .game
            .cards_in_zone(ZoneType::Library, target_player)
            .len();
        if lib_len == 0 {
            return;
        }

        let mut found = Vec::new();
        let mut revealed = Vec::new();
        let mut rest_cmc = crate::parsing::raw_get(&sa.ability_text, "MinTotalCMC")
            .map(|raw| crate::svar::resolve_numeric_value(ctx.game, sa, raw, 0));

        // Walk from top of library down
        let lib_cards: Vec<_> = ctx
            .game
            .cards_in_zone(ZoneType::Library, target_player)
            .to_vec();
        // Library is stored bottom→top, so iterate from end (top) backwards
        for &cid in lib_cards.iter().rev() {
            if found.len() >= amount {
                break;
            }
            revealed.push(cid);
            if let Some(rest) = rest_cmc.as_mut() {
                *rest -= ctx.game.card(cid).mana_cost.cmc();
                if *rest <= 0 {
                    break;
                }
                continue;
            }
            let card = ctx.game.card(cid);
            let matches = match (valid_selector, sa.source) {
                (Some(selector), Some(source_id)) => {
                    valid_filter::matches_valid_card_selector_in_game(
                        selector,
                        card,
                        ctx.game.card(source_id),
                        ctx.game,
                    )
                }
                _ => matches_change_type(card, valid_filter, &[]),
            };
            if matches {
                found.push(cid);
                if let Some(source_id) = sa.source {
                    if crate::parsing::raw_has_key(&sa.ability_text, keys::FORGET_OTHER_REMEMBERED)
                    {
                        ctx.game.card_mut(source_id).clear_remembered();
                    }
                    if sa.ir.remember_found {
                        ctx.game.card_mut(source_id).add_remembered_card(cid);
                    }
                    if sa.ir.imprint_found {
                        ctx.game.card_mut(source_id).add_imprinted_card(cid);
                    }
                }
            }
        }
        if let Some(found_dest) = found_dest
            .filter(|_| crate::parsing::raw_has_key(&sa.ability_text, "OptionalFoundMove"))
        {
            let mut kept = Vec::new();
            for &cid in &found {
                ctx.agents[target_player.index()].snapshot_state(ctx.game, ctx.mana_pools);
                if ctx.agents[target_player.index()].confirm_action(
                    target_player,
                    None,
                    &format!("Do you want to put that card to {found_dest:?}?"),
                    &[],
                    sa.source,
                    sa.api,
                ) {
                    kept.push(cid);
                }
            }
            found = kept;
        }
        let mut rest: Vec<_> = if found_dest.is_some() {
            revealed
                .iter()
                .copied()
                .filter(|cid| !found.contains(cid))
                .collect()
        } else {
            revealed.clone()
        };
        if let Some(source_id) = sa.source {
            if sa.ir.imprint_revealed {
                ctx.game
                    .card_mut(source_id)
                    .add_imprinted_cards(rest.iter().copied());
            }
            if sa.ir.remember_revealed {
                ctx.game
                    .card_mut(source_id)
                    .add_remembered_cards(rest.iter().copied());
            }
        }

        // Remove found + rest cards from library
        let removed: Vec<_> = revealed.to_vec();
        for card_id in removed {
            ctx.game
                .remove_card_from_zone(ZoneType::Library, target_player, card_id);
        }

        // Move found cards to destination
        if let Some(found_dest) = found_dest {
            for &id in &found {
                let owner = ctx.game.card(id).owner;
                let dest_owner = if found_dest == ZoneType::Battlefield {
                    sa.activating_player
                } else {
                    owner
                };
                ctx.move_card(id, found_dest, dest_owner);
                if found_dest == ZoneType::Exile {
                    if let Some(source_id) = sa.source {
                        ctx.game.card_mut(source_id).add_exiled_card(id);
                    }
                }
                if sa.ir.tapped && found_dest == ZoneType::Battlefield {
                    ctx.game.card_mut(id).tapped = true;
                }
                if found_dest == ZoneType::Battlefield {
                    let _ = super::add_to_combat(ctx, sa, id, keys::ATTACKING);
                }
                emit_zone_trigger(ctx.trigger_handler, id, ZoneType::Library, found_dest);
            }
        }

        let shuffle = crate::parsing::raw_has_key(&sa.ability_text, "Shuffle");
        let random_order = crate::parsing::raw_has_key(&sa.ability_text, "RevealRandomOrder");
        if random_order && rest.len() > 1 {
            for i in (1..rest.len()).rev() {
                let j = ctx.rng.next_int((i + 1) as i32) as usize;
                rest.swap(i, j);
            }
        }
        let sequential = found_dest == Some(revealed_dest);
        // The dig took every revealed card off the library up front, where Java only looks at
        // them, so "don't move them" has to put them back in the order they were seen.
        if crate::parsing::raw_has_key(&sa.ability_text, "NoMoveRevealed") {
            for &id in rest.iter().rev() {
                let owner = ctx.game.card(id).owner;
                ctx.game.add_card_to_zone(ZoneType::Library, owner, id);
                ctx.game.card_mut(id).set_zone(ZoneType::Library);
            }
            continue;
        }

        let mut final_dest = revealed_dest;
        let mut final_pos = library_position(ctx, sa, "RevealedLibraryPosition");
        if !sequential && found.len() < amount {
            if let Some(none_found) =
                crate::parsing::raw_get(&sa.ability_text, "NoneFoundDestination")
                    .and_then(|raw| ZoneType::from_str_compat(raw.trim()))
            {
                final_dest = none_found;
                final_pos = library_position(ctx, sa, "NoneFoundLibraryPosition");
            }
        }

        let known_dest = !matches!(final_dest, ZoneType::Library | ZoneType::Hand);
        if !sequential
            && (known_dest || (final_dest == ZoneType::Library && !shuffle && !random_order))
            && rest.len() >= 2
        {
            ctx.agents[target_player.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let ordered = ctx.agents[target_player.index()].order_move_to_zone_list(
                ctx.game,
                target_player,
                &rest,
                final_dest,
            );
            if ordered.len() == rest.len() && rest.iter().all(|id| ordered.contains(id)) {
                rest = ordered;
            }
        }

        for &id in &rest {
            let owner = ctx.game.card(id).owner;
            if final_dest == ZoneType::Library {
                if final_pos < 0 {
                    ctx.game
                        .add_card_to_zone_bottom(ZoneType::Library, owner, id);
                } else {
                    ctx.game.add_card_to_zone(ZoneType::Library, owner, id);
                }
                ctx.game.card_mut(id).set_zone(ZoneType::Library);
            } else {
                ctx.move_card(id, final_dest, owner);
                if final_dest == ZoneType::Exile {
                    if let Some(source_id) = sa.source {
                        ctx.game.card_mut(source_id).add_exiled_card(id);
                    }
                }
                emit_zone_trigger(ctx.trigger_handler, id, ZoneType::Library, final_dest);
            }
        }
    }
}

fn library_position(ctx: &EffectContext, sa: &crate::spellability::SpellAbility, key: &str) -> i32 {
    crate::parsing::raw_get(&sa.ability_text, key)
        .map(|raw| super::resolve_numeric_value(ctx.game, sa, raw, 0))
        .unwrap_or(0)
}
