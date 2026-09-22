//! Manifest effect — put cards onto the battlefield face-down as 2/2 creatures.
//!
//! Ported from Java's `ManifestEffect.java` + `ManifestBaseEffect.java`.
//!
//! Manifest: Take the top card of a player's library (or chosen cards),
//! turn it face-down, and put it onto the battlefield as a 2/2 creature.
//! The card can be turned face-up by paying its mana cost if it's a creature.

use forge_foundation::ZoneType;

use super::manifest_base_effect::{manifest_target_cards, parse_manifest_params};
use super::{emit_zone_trigger, EffectContext};
use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ManifestEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ManifestEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let manifest_params = parse_manifest_params(ctx, sa);
    let amount = manifest_params.amount;
    let controller = sa.activating_player;

    // DefinedPlayer$ → SA-aware resolution (Targeted preserves empty lists).
    let players = if let Some(def) = sa.defined_player() {
        crate::ability::ability_utils::resolve_defined_players_with_sa(
            def, sa, controller, ctx.game,
        )
    } else {
        vec![controller]
    };

    for pid in players {
        manifest_for_player(ctx, sa, pid, amount);
    }
}

/// Manifest N cards for a given player.
fn manifest_for_player(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    player: PlayerId,
    amount: usize,
) {
    let Some(cards_to_manifest) = manifest_target_cards(ctx, sa, player, amount) else {
        return;
    };

    // Manifest each card one at a time (CR 701.34d)
    for card_id in cards_to_manifest {
        manifest_single_card(ctx, sa, card_id, player);
    }
}

/// Manifest a single card: turn face-down, put on battlefield as 2/2.
pub(super) fn manifest_single_card(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    card_id: CardId,
    player: PlayerId,
) {
    let old_zone = ctx.game.card(card_id).zone;

    // Turn face down
    ctx.game.card_mut(card_id).set_face_down(true);
    ctx.game.card_mut(card_id).set_original_state_as_face_down();
    ctx.game.card_mut(card_id).set_manifested(true);
    crate::card::card_factory_util::ability_turn_face_up(ctx.game.card_mut(card_id), "ManifestUp");

    // Move to battlefield under the player's control
    ctx.game.card_mut(card_id).set_controller(player);
    ctx.move_card(card_id, ZoneType::Battlefield, player);

    ctx.trigger_handler
        .register_active_trigger(ctx.game, card_id);

    // RememberManifested$
    if sa.ir.remember_manifested {
        if let Some(source_id) = sa.source {
            ctx.game.card_mut(source_id).add_remembered_card(card_id);
        }
    }

    emit_zone_trigger(
        ctx.trigger_handler,
        card_id,
        old_zone,
        ZoneType::Battlefield,
    );
}
