//! Vote effect — Council's Dilemma and Will of the Council voting.
//!
//! Ported from Java's `VoteEffect.java`.

use forge_foundation::ZoneType;

use super::helpers::matches_valid_cards_for_sa;
use super::EffectContext;
use crate::event::RunParams;
use crate::ids::{CardId, PlayerId};
use crate::parsing::Params;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

#[derive(Clone, PartialEq)]
enum VoteOption {
    Ability(usize),
    Card(CardId),
    Player(PlayerId),
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `VoteEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(VoteEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(host) = sa.source else {
        return;
    };
    let params = Params::from_raw(&sa.ability_text);
    let activator = sa.activating_player;
    let mut voters =
        crate::ability::spell_ability_effect::get_defined_players_or_targeted(ctx.game, sa);
    let other = params.get("VotePlayer") == Some("Other");

    let mut choice_abilities: Vec<SpellAbility> = Vec::new();
    let mut vote_type: Vec<VoteOption> = Vec::new();
    if let Some(choices) = params.get("Choices") {
        for name in choices.split(',').map(str::trim) {
            if let Some(text) = ctx.game.card(host).get_s_var(name).map(str::to_string) {
                vote_type.push(VoteOption::Ability(choice_abilities.len()));
                choice_abilities.push(crate::spellability::build_spell_ability(
                    ctx.game, host, &text, activator,
                ));
            }
        }
    } else if let Some(valid) = params.get("VoteCard") {
        let zone = params
            .get("Zone")
            .and_then(ZoneType::from_str_compat)
            .unwrap_or(ZoneType::Battlefield);
        for player in ctx.game.players.iter().map(|p| p.id).collect::<Vec<_>>() {
            for &cid in ctx.game.cards_in_zone(zone, player) {
                if matches_valid_cards_for_sa(ctx.game, sa, ctx.game.card(cid), None, valid) {
                    vote_type.push(VoteOption::Card(cid));
                }
            }
        }
    } else if let Some(defined) = params.get("VotePlayer") {
        let defined = if other { "Player" } else { defined };
        vote_type.extend(
            crate::ability::ability_utils::resolve_defined_players_with_sa(
                defined, sa, activator, ctx.game,
            )
            .into_iter()
            .map(VoteOption::Player),
        );
    }
    if vote_type.is_empty() {
        return;
    }

    if let Some(pos) = voters.iter().position(|&p| p == activator) {
        voters.rotate_left(pos);
    }

    let mut votes: Vec<(VoteOption, Vec<PlayerId>)> = Vec::new();
    for voter in voters {
        if !ctx.game.player(voter).is_alive() {
            continue;
        }
        let mut options = vote_type.clone();
        if other {
            options.retain(|o| *o != VoteOption::Player(voter));
            if options.is_empty() {
                continue;
            }
        }
        ctx.agents[voter.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let extra = ctx.agents[voter.index()]
            .choose_number(
                voter,
                sa.source,
                "How many additional votes do you want?",
                None,
                0,
                0,
            )
            .unwrap_or(0);
        let labels: Vec<String> = options
            .iter()
            .map(|o| option_label(ctx, &choice_abilities, o))
            .collect();
        for _ in 0..(1 + extra) {
            let Some(index) = ctx.agents[voter.index()].vote(voter, &labels, params.has("UpTo"))
            else {
                continue;
            };
            let Some(option) = options.get(index).cloned() else {
                continue;
            };
            match votes.iter_mut().find(|(o, _)| *o == option) {
                Some((_, list)) => list.push(voter),
                None => votes.push((option, vec![voter])),
            }
        }
    }

    let all_votes: Vec<(String, Vec<PlayerId>)> = votes
        .iter()
        .map(|(o, v)| (option_label(ctx, &choice_abilities, o), v.clone()))
        .collect();
    ctx.trigger_handler.run_trigger(
        TriggerType::Vote,
        RunParams {
            all_votes: Some(all_votes),
            ..Default::default()
        },
        false,
    );

    if params.has("EachVote") {
        for (option, voters) in &votes {
            let VoteOption::Ability(index) = option else {
                continue;
            };
            for &player in voters {
                ctx.game.card_mut(host).add_remembered_player(player);
                super::effect_resolver::resolve_effect_chain(ctx, choice_abilities[*index].clone());
                ctx.game
                    .card_mut(host)
                    .remembered_players
                    .retain(|p| *p != player);
            }
        }
        return;
    }

    let store_vote_num = params.has("StoreVoteNum");
    let has_choices = params.has("Choices");
    let mut sub_abilities: Vec<(SpellAbility, Option<usize>)> = Vec::new();
    if store_vote_num && has_choices {
        for option in &vote_type {
            let VoteOption::Ability(index) = option else {
                continue;
            };
            let count = votes
                .iter()
                .find(|(o, _)| o == option)
                .map_or(0, |(_, v)| v.len());
            sub_abilities.push((choice_abilities[*index].clone(), Some(count)));
        }
    } else {
        let most = most_votes(&votes);
        if most.len() > 1 && params.has("VoteTiedAbility") {
            if let Some(sub) = sa.additional_ability(ctx.game, "VoteTiedAbility") {
                sub_abilities.push((sub, None));
            }
        } else if params.has("VoteSubAbility") {
            let card = ctx.game.card_mut(host);
            for option in &most {
                match option {
                    VoteOption::Card(cid) => card.add_remembered_card(*cid),
                    VoteOption::Player(pid) => card.add_remembered_player(*pid),
                    VoteOption::Ability(_) => {}
                }
            }
            if let Some(sub) = sa.additional_ability(ctx.game, "VoteSubAbility") {
                sub_abilities.push((sub, None));
            }
        } else if has_choices {
            for option in &most {
                if let VoteOption::Ability(index) = option {
                    sub_abilities.push((choice_abilities[*index].clone(), None));
                }
            }
        }
    }

    if store_vote_num && !has_choices {
        for option in &vote_type {
            let count = votes
                .iter()
                .find(|(o, _)| o == option)
                .map_or(0, |(_, v)| v.len());
            let label = option_label(ctx, &choice_abilities, option);
            ctx.game
                .card_mut(host)
                .set_s_var(format!("VoteNum{label}"), format!("Number${count}"));
        }
    } else {
        for (sub, vote_num) in sub_abilities {
            if let Some(count) = vote_num {
                ctx.game
                    .card_mut(host)
                    .set_s_var("VoteNum".to_string(), format!("Number${count}"));
            }
            super::effect_resolver::resolve_effect_chain(ctx, sub);
        }
    }
    if params.has("VoteSubAbility") {
        ctx.game.card_mut(host).clear_remembered();
    }
    if params.has("RememberVotedObjects") {
        let card = ctx.game.card_mut(host);
        for (option, _) in &votes {
            match option {
                VoteOption::Card(cid) => card.add_remembered_card(*cid),
                VoteOption::Player(pid) => card.add_remembered_player(*pid),
                VoteOption::Ability(_) => {}
            }
        }
    }
}

fn most_votes(votes: &[(VoteOption, Vec<PlayerId>)]) -> Vec<VoteOption> {
    let mut most = Vec::new();
    let mut amount = 0;
    for (option, voters) in votes {
        if voters.len() == amount {
            most.push(option.clone());
        } else if voters.len() > amount {
            amount = voters.len();
            most.clear();
            most.push(option.clone());
        }
    }
    most
}

fn option_label(ctx: &EffectContext, choices: &[SpellAbility], option: &VoteOption) -> String {
    match option {
        VoteOption::Ability(index) => choices[*index].description.clone(),
        VoteOption::Card(cid) => ctx.game.card(*cid).card_name.clone(),
        VoteOption::Player(pid) => ctx.game.player(*pid).name.clone(),
    }
}
