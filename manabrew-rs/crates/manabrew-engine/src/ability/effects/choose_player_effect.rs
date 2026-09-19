use super::EffectContext;
use crate::agent::GameEntity;
use crate::parsing::keys;

/// `SP$ ChoosePlayer` — the activating player chooses a player.
/// Stores the result in `source.chosen_player` for subsequent effects.
///
/// Mirrors Java's `ChoosePlayerEffect.java`.
///
/// # Card script examples
/// ```text
/// A:SP$ ChoosePlayer | Defined$ You
/// ```
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ChoosePlayerEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ChoosePlayerEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let controller = sa.activating_player;
    let choosers = crate::ability::spell_ability_effect::get_target_players(ctx.game, sa);

    let valid_players: Vec<_> = if let Some(choices) = sa.ir.choices.as_deref() {
        crate::ability::ability_utils::resolve_defined_players_with_sa(
            choices, sa, controller, ctx.game,
        )
        .into_iter()
        .filter(|&pid| ctx.game.player(pid).is_alive())
        .collect()
    } else {
        // Match Java getPlayersInTurnOrder() ordering while excluding players
        // no longer in game.
        ctx.game
            .player_order
            .iter()
            .copied()
            .filter(|&pid| ctx.game.player(pid).is_alive())
            .collect()
    };

    for chooser in choosers {
        if !ctx.game.player(chooser).is_alive() {
            continue;
        }
        let chosen = if valid_players.is_empty() {
            None
        } else if sa.ir.random {
            let index = if valid_players.len() == 1 {
                0
            } else {
                ctx.rng.next_int(valid_players.len() as i32) as usize
            };
            Some(valid_players[index])
        } else {
            let entities: Vec<GameEntity> = valid_players
                .iter()
                .copied()
                .map(GameEntity::Player)
                .collect();
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            match ctx.agents[chooser.index()].choose_single_entity_for_effect(
                chooser,
                &entities,
                sa.ir.optional,
            ) {
                Some(GameEntity::Player(pid)) => Some(pid),
                _ => None,
            }
        };

        if let Some(chosen_pid) = chosen {
            if let Some(source_id) = sa.source {
                ctx.game.card_mut(source_id).set_chosen_player(
                    Some(chosen_pid),
                    Some(chooser),
                    !sa.ir.secretly,
                );
                if sa.param_is_true(keys::REMEMBER_CHOSEN) {
                    ctx.game
                        .card_mut(source_id)
                        .add_remembered_player(chosen_pid);
                }
            }
        }
    }
}
