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

    let combat = sa.param_is_true(crate::parsing::keys::COMBAT);
    for target_player in targets {
        let command = crate::phase::PhaseCommand::AddController {
            player: target_player,
            controller,
            combat,
        };
        if combat {
            ctx.game
                .begin_of_combat
                .add_until(Some(target_player), command);
        } else {
            ctx.game.cleanup.add_until(Some(target_player), command);
        }
    }
}
