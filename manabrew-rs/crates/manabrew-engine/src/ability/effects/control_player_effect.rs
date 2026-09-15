//! ControlPlayer effect — take control of another player's turn (Mindslaver).
//!
//! Ported 1:1 from Java's `ControlPlayerEffect.java`.
//! You control target player during their next turn. (CR 800.4b)
//! The controlled player's decisions are made by the controller.

use super::EffectContext;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ControlPlayerEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ControlPlayerEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let controller_def = sa.ir.controller_text.as_deref().unwrap_or("You");
    let controller = super::resolve_defined_players(controller_def, sa.activating_player, ctx.game)
        .into_iter()
        .next()
        .unwrap_or(sa.activating_player);

    let targets = if let Some(pid) = sa.target_chosen.target_player {
        vec![pid]
    } else if let Some(def) = sa.defined_player() {
        super::resolve_defined_players(def, sa.activating_player, ctx.game)
    } else {
        vec![ctx.game.opponent_of(sa.activating_player)]
    };

    for target_player in targets {
        // Set the controlled_by field on the target player
        // This will be checked by the game loop to route decisions
        // through the controller's agent instead of the target's agent
        ctx.game
            .player_set_controlled_by(target_player, Some(controller));
    }
}
