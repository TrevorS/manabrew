use forge_foundation::ZoneType;

use super::EffectContext;
use crate::event::RunParams;
use crate::trigger::TriggerType;

/// Revert a scheduled control-gain. Mirrors Java `ControlGainEffect`'s
/// `GameCommand.run()`: restore `original_controller_eot`, drop the
/// `lose_control_condition`, and clear granted keywords. Zone guard — if the
/// card has already left the battlefield, the scheduler caller is expected to
/// have handled cleanup via `leaves_play_hook`.
pub fn run(game: &mut crate::game::GameState, card_id: crate::ids::CardId) {
    if game.card(card_id).zone != ZoneType::Battlefield {
        return;
    }
    revert(game, card_id);
}

fn revert(game: &mut crate::game::GameState, card_id: crate::ids::CardId) {
    if let Some(original) = game.card(card_id).original_controller_eot {
        game.change_controller(card_id, original);
        game.card_mut(card_id).set_original_controller_eot(None);
    }
    game.card_mut(card_id).lose_control_condition = None;
    game.card_mut(card_id).clear_granted_keywords();
}

/// Hook invoked whenever a card untaps — reverts the steal if the card was
/// scheduled with `LoseControlCondition::NextUntap`.
pub fn untap_hook(game: &mut crate::game::GameState, card_id: crate::ids::CardId) {
    if game.card(card_id).lose_control_condition
        == Some(crate::card::LoseControlCondition::NextUntap)
    {
        revert(game, card_id);
    }
}

/// Hook invoked at end of combat — reverts steals scheduled with
/// `LoseControlCondition::EndOfCombat`.
pub fn end_of_combat_hook(game: &mut crate::game::GameState) {
    let targets: Vec<crate::ids::CardId> = game
        .cards
        .iter()
        .filter(|c| {
            c.lose_control_condition == Some(crate::card::LoseControlCondition::EndOfCombat)
        })
        .map(|c| c.id)
        .collect();
    for cid in targets {
        if game.card(cid).zone == ZoneType::Battlefield {
            revert(game, cid);
        }
    }
}

/// Hook invoked as a permanent leaves the battlefield — reverts scheduled
/// `LoseControlCondition::LeavesPlay` commands. The card is technically
/// already in limbo when this fires, so we just clear the schedule.
pub fn leaves_play_hook(game: &mut crate::game::GameState, card_id: crate::ids::CardId) {
    if game.card(card_id).lose_control_condition
        == Some(crate::card::LoseControlCondition::LeavesPlay)
    {
        game.card_mut(card_id).lose_control_condition = None;
        game.card_mut(card_id).set_original_controller_eot(None);
    }
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
        run(&mut game, goblin);
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
    let lose: Vec<&str> = crate::parsing::raw_get(&sa.ability_text, "LoseControl")
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

    // Schedule the controller-return GameCommand based on the LoseControl$
    // variant. Only record original_controller on the first steal so repeated
    // steal → steal-back → EOT reverts to the pre-chain controller.
    let already_scheduled = ctx.game.card(target_card).original_controller_eot.is_some();
    if let Some(cond) = sa.ir.lose_control {
        if !already_scheduled {
            ctx.game
                .card_mut(target_card)
                .set_original_controller_eot(Some(old_controller));
        }
        ctx.game.card_mut(target_card).lose_control_condition = Some(cond);
    }

    // Handle Untap parameter
    if sa.ir.untap_on_resolve {
        ctx.game.untap(target_card);
    }

    // Handle AddKWs parameter (add keywords)
    if let Some(kws_str) = sa.ir.add_kws.as_deref() {
        let keywords: Vec<String> = kws_str.split(" & ").map(|s| s.to_string()).collect();
        for kw in keywords {
            ctx.game.card_mut(target_card).add_pump_keyword(&kw);
        }
    }

    if remember {
        ctx.game.card_mut(source).add_remembered_card(target_card);
    }
    if forget {
        ctx.game.card_mut(source).remove_remembered(target_card);
    }
}
