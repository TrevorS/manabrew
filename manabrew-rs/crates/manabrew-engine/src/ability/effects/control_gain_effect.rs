use forge_foundation::ZoneType;

use super::EffectContext;
use crate::event::RunParams;
use crate::trigger::TriggerType;

/// Mirrors the command `ControlGainEffect.getLoseControlCommand` builds.
pub fn lose_control(
    game: &mut crate::game::GameState,
    card_id: crate::ids::CardId,
    timestamp: i64,
) {
    if game.card(card_id).zone != ZoneType::Battlefield
        || !game.card_mut(card_id).remove_temp_controller(timestamp)
    {
        return;
    }
    let card = game.card(card_id);
    let controller = card
        .temp_controllers
        .last()
        .map(|&(_, player)| player)
        .or(card.original_controller_eot)
        .unwrap_or(card.owner);
    if card.temp_controllers.is_empty() {
        game.card_mut(card_id).set_original_controller_eot(None);
    }
    game.change_controller(card_id, controller);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ability::spell_ability_effect::SpellAbilityEffect;
    use crate::agent::{PassAgent, PlayerAgent};
    use crate::card::Card;
    use crate::game::GameState;
    use crate::ids::{CardId, PlayerId};
    use crate::mana::ManaPool;
    use crate::spellability::SpellAbility;
    use crate::trigger::TriggerHandler;
    use crate::HashMap;
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost, ZoneType};

    fn creature(owner: PlayerId, name: &str) -> Card {
        Card::new(
            CardId(0),
            name.to_string(),
            owner,
            CardTypeLine::parse("Creature Goblin"),
            ManaCost::parse("R"),
            ColorSet::RED,
            Some(1),
            Some(1),
            vec![],
            vec![],
        )
    }

    #[test]
    fn repeated_eot_control_gain_restores_first_controller() {
        let p0 = PlayerId(0);
        let p1 = PlayerId(1);
        let mut game = GameState::new(&["Alice", "Bob"], 20);
        let goblin = game.create_card(creature(p0, "Raging Goblin"));
        game.move_card(goblin, ZoneType::Battlefield, p0);

        let mut agents: Vec<Box<dyn PlayerAgent>> = vec![Box::new(PassAgent), Box::new(PassAgent)];
        let mut trigger_handler = TriggerHandler::new();
        let templates = HashMap::default();
        let token_art_variants = HashMap::default();
        let token_fallback = HashMap::default();
        let edition_dates = HashMap::default();
        let mut mana_pools = vec![ManaPool::new(), ManaPool::new()];
        let mut rng = crate::game_rng::ThreadRngAdapter::default();

        {
            let mut ctx = EffectContext {
                game: &mut game,
                combat: None,
                agents: &mut agents,
                trigger_handler: &mut trigger_handler,
                token_templates: &templates,
                token_art_variants: &token_art_variants,
                token_fallback: &token_fallback,
                edition_dates: &edition_dates,
                mana_pools: &mut mana_pools,
                parent_target_card: None,
                rng: &mut rng,
            };

            let mut steal = SpellAbility::new_simple(
                Some(goblin),
                p1,
                "SP$ GainControl | ValidTgts$ Creature.OppCtrl | LoseControl$ EOT",
            );
            steal.target_chosen.target_card = Some(goblin);
            ControlGainEffect::resolve(&mut ctx, &steal);
            assert_eq!(ctx.game.card(goblin).controller, p1);

            let mut steal_back = SpellAbility::new_simple(
                Some(goblin),
                p0,
                "SP$ GainControl | ValidTgts$ Creature.OppCtrl | LoseControl$ EOT",
            );
            steal_back.target_chosen.target_card = Some(goblin);
            ControlGainEffect::resolve(&mut ctx, &steal_back);
            assert_eq!(ctx.game.card(goblin).controller, p0);
        }

        assert_eq!(game.card(goblin).original_controller_eot, Some(p0));
        let mut rng = crate::game_rng::ThreadRngAdapter::default();
        for command in game.end_of_turn.execute_until(None) {
            command.run(&mut game, &mut rng);
        }
        assert_eq!(game.card(goblin).controller, p0);
        assert_eq!(game.card(goblin).original_controller_eot, None);
    }
}

/// SP$ ControlGain — gain control of target permanent until end of turn or permanently.
///
/// Mirrors Java's `ControlGainEffect.resolve()`.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ControlGainEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ControlGainEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(source) = sa.source else {
        return;
    };
    let activator = sa.activating_player;
    let remember = crate::parsing::raw_has_key(&sa.ability_text, "RememberControlled");
    let forget = crate::parsing::raw_has_key(&sa.ability_text, "ForgetControlled");
    let lose: Vec<&str> =
        crate::parsing::raw_get(&sa.ability_text, crate::parsing::keys::LOSE_CONTROL)
            .map(|raw| raw.split(',').map(str::trim).collect())
            .unwrap_or_default();

    let controllers = match crate::parsing::raw_get(&sa.ability_text, "NewController") {
        Some(defined) => crate::ability::ability_utils::resolve_defined_players_with_sa(
            defined, sa, activator, ctx.game,
        ),
        None if sa.uses_targeting() => sa.target_chosen.all_target_players(),
        None => vec![activator],
    };
    let new_controller = controllers.first().copied().unwrap_or(activator);

    let battlefield: Vec<crate::ids::CardId> = ctx
        .game
        .player_order
        .iter()
        .flat_map(|&pid| {
            ctx.game
                .cards_in_zone(ZoneType::Battlefield, pid)
                .iter()
                .copied()
        })
        .collect();
    let tgt_cards = if let Some(choices) = crate::parsing::raw_get(&sa.ability_text, "Choices") {
        let chooser = crate::parsing::raw_get(&sa.ability_text, "Chooser")
            .and_then(|defined| {
                crate::ability::ability_utils::resolve_defined_players_with_sa(
                    defined, sa, activator, ctx.game,
                )
                .into_iter()
                .next()
            })
            .unwrap_or(activator);
        let selector = crate::parsing::cached_compiled_selector(choices);
        let valid: Vec<_> = battlefield
            .into_iter()
            .filter(|&cid| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    Some(&selector),
                    choices,
                )
            })
            .collect();
        if valid.is_empty() {
            return;
        }
        ctx.agents[chooser.index()].choose_cards_for_effect(chooser, &valid, 1, 1)
    } else if let Some(all_valid) = sa.ir.all_valid_selector.as_ref() {
        battlefield
            .into_iter()
            .filter(|&cid| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    Some(all_valid),
                    "",
                )
            })
            .collect()
    } else {
        crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa)
    };

    if lose.contains(&"LeavesPlay") && ctx.game.card(source).zone != ZoneType::Battlefield {
        return;
    }
    if lose.contains(&"LoseControl") && ctx.game.card(source).controller != activator {
        return;
    }
    if lose.contains(&"Untap") && !ctx.game.card(source).tapped {
        return;
    }

    for target_card in tgt_cards {
        gain_control_of(
            ctx,
            sa,
            source,
            target_card,
            new_controller,
            remember,
            forget,
            &lose,
        );
    }
}

fn gain_control_of(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    source: crate::ids::CardId,
    target_card: crate::ids::CardId,
    new_controller: crate::ids::PlayerId,
    remember: bool,
    forget: bool,
    lose: &[&str],
) {
    if ctx.game.card(target_card).zone != ZoneType::Battlefield
        || !ctx
            .game
            .card(target_card)
            .can_be_controlled_by(new_controller)
        || ctx.game.card(target_card).phased_out
    {
        return;
    }
    if sa.ir.optional {
        let activator = sa.activating_player;
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[activator.index()].confirm_action(
            activator,
            None,
            "Do you want to gain control of that card?",
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }

    // Change controller
    let old_controller = ctx.game.card(target_card).controller;
    ctx.game.change_controller(target_card, new_controller);

    // Fire ChangesController trigger (mirrors Java GameAction.doChangeController)
    if old_controller != new_controller {
        ctx.trigger_handler.run_trigger(
            TriggerType::ChangesController,
            RunParams {
                card: Some(target_card),
                player: Some(new_controller),
                original_controller: Some(old_controller),
                ..Default::default()
            },
            false,
        );
    }

    let timestamp = ctx.game.next_timestamp() as i64;
    if ctx.game.card(target_card).temp_controllers.is_empty() {
        ctx.game
            .card_mut(target_card)
            .set_original_controller_eot(Some(old_controller));
    }
    ctx.game
        .card_mut(target_card)
        .add_temp_controller(new_controller, timestamp);
    let lose_control = crate::phase::PhaseCommand::LoseControl {
        card: target_card,
        timestamp,
    };
    if lose.contains(&"LeavesPlay") && source != target_card {
        ctx.game
            .leaves_play_commands
            .push((source, lose_control.clone()));
    }
    if lose.contains(&"Untap") {
        ctx.game.untap_commands.push((source, lose_control.clone()));
    }
    if lose.contains(&"LoseControl") {
        ctx.game
            .change_controller_commands
            .push((source, lose_control.clone()));
    }
    if lose.contains(&"EOT") {
        ctx.game.end_of_turn.add_until(None, lose_control.clone());
    }
    if lose.contains(&"EndOfCombat") {
        ctx.game.end_of_combat.add_until(None, lose_control.clone());
    }
    if lose.contains(&"UntilTheEndOfYourNextTurn") {
        let activator = sa.activating_player;
        if ctx.game.active_player() == activator {
            ctx.game
                .end_of_turn
                .register_until_end(activator, lose_control);
        } else {
            ctx.game.end_of_turn.add_until_end(activator, lose_control);
        }
    }

    // Handle Untap parameter
    if sa.ir.untap_on_resolve {
        ctx.game.untap(target_card);
    }

    // Handle AddKWs parameter (add keywords)
    if let Some(kws_str) = sa.ir.add_kws.as_deref() {
        let keywords: Vec<String> = kws_str.split(" & ").map(|s| s.to_string()).collect();
        let timestamp = ctx.game.next_timestamp();
        for kw in keywords {
            ctx.game
                .card_mut(target_card)
                .add_pump_keyword(&kw, timestamp);
        }
    }

    if remember {
        ctx.game.card_mut(source).add_remembered_card(target_card);
    }
    if forget {
        ctx.game.card_mut(source).remove_remembered(target_card);
    }
}
