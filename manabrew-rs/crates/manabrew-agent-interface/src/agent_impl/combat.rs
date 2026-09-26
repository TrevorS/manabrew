use manabrew_engine::agent::{CombatCostAction, ManaAbilityOption};
use manabrew_engine::combat::attack_requirement::compute_attack_requirements_with_defenders;
use manabrew_engine::combat::{self, combat_util, CombatState, DefenderId, LureType};
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::staticability::static_ability_cant_attack_block;
use manabrew_protocol::prompts::choose_attackers::{AttackerOptionDto, ChooseAttackersInput};
use manabrew_protocol::prompts::choose_blockers::{
    BlockRequirementDto, BlockableAttackerDto, ChooseBlockersInput,
};

use crate::game_view_dto::{CardDto, GameViewDtoExt, TargetingIntent};
use crate::ids_codec::{card_id_str, parse_card_id, player_id_str};
use crate::mana_action_id::parse_tap_action_id;
use crate::prompt::*;

use super::costs::mana_payment_actions;
use super::{parse_express_mana_choice, PromptAgent, Responder};

fn defender_id_str(defender: DefenderId) -> String {
    match defender {
        DefenderId::Player(pid) => player_id_str(pid),
        DefenderId::Permanent(cid) => card_id_str(cid),
    }
}

fn fallback_combat_assignment(
    blockers_in_order: &[CardId],
    defender: Option<DefenderId>,
    total_damage: i32,
) -> Vec<(Option<CardId>, i32)> {
    if total_damage <= 0 {
        return Vec::new();
    }
    if let Some(first) = blockers_in_order.first().copied() {
        return vec![(Some(first), total_damage)];
    }
    if defender.is_some() {
        return vec![(None, total_damage)];
    }
    Vec::new()
}

pub(super) fn choose_attackers<T: Responder>(
    agent: &mut PromptAgent<T>,
    _player: PlayerId,
    available: &[CardId],
    possible_defenders: &[DefenderId],
) -> Vec<(CardId, DefenderId)> {
    let game = agent
        .combat_game
        .take()
        .expect("snapshot_state runs before the attack declaration");
    let requirements =
        compute_attack_requirements_with_defenders(&game, available, possible_defenders);
    let attackers = requirements
        .iter()
        .map(|requirement| {
            let legal: Vec<DefenderId> = possible_defenders
                .iter()
                .copied()
                .filter(|&defender| {
                    combat_util::can_attack_defender(&game, requirement.attacker, defender)
                })
                .collect();
            let credit = |defender: &DefenderId| {
                requirement
                    .defender_specific
                    .get(defender)
                    .copied()
                    .unwrap_or(0)
            };
            let best = legal.iter().map(credit).max().unwrap_or(0);
            let must_attack_target_ids = requirement.has_requirement().then(|| {
                legal
                    .iter()
                    .filter(|defender| credit(defender) == best)
                    .copied()
                    .map(defender_id_str)
                    .collect()
            });
            AttackerOptionDto {
                attacker_id: card_id_str(requirement.attacker),
                valid_target_ids: legal.into_iter().map(defender_id_str).collect(),
                must_attack: must_attack_target_ids.is_some(),
                must_attack_target_ids,
            }
        })
        .collect();
    agent.send_prompt(
        PromptInput::ChooseAttackers(ChooseAttackersInput {
            attackers,
            attack_targets: PromptAgent::<T>::attack_targets_to_dtos(possible_defenders),
        }),
        None,
    );
    match agent.recv_action() {
        PromptOutput::ChooseAttackers(ChooseAttackersOutput::DeclareAttackers { assignments }) => {
            assignments
                .iter()
                .filter_map(|a| {
                    let attacker = parse_card_id(&a.attacker_id)?;
                    let defender =
                        PromptAgent::<T>::parse_defender_id(&a.target_id, possible_defenders)?;
                    Some((attacker, defender))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn choose_blockers<T: Responder>(
    agent: &mut PromptAgent<T>,
    player: PlayerId,
    attackers: &[CardId],
    available_blockers: &[CardId],
    _max_blockers: Option<usize>,
) -> Vec<(CardId, CardId)> {
    let game = agent
        .combat_game
        .take()
        .expect("snapshot_state runs before the block declaration");
    let mut combat = CombatState::new();
    for &attacker in attackers {
        combat.declare_attacker(attacker, DefenderId::Player(player), 0);
    }
    let can_block = |blocker: CardId, attacker: CardId| {
        combat::can_creature_block(&game, blocker, attacker)
            && !combat_util::lure_forbids_block(&game, &combat, attacker, blocker)
    };
    let blockers: Vec<CardId> = available_blockers
        .iter()
        .copied()
        .filter(|&blocker| {
            attackers
                .iter()
                .any(|&attacker| can_block(blocker, attacker))
        })
        .collect();
    let attacker_options = attackers
        .iter()
        .map(|&attacker| {
            let (min, max) = static_ability_cant_attack_block::get_min_max_blocker(
                &game,
                &game.cards,
                game.card(attacker),
                player,
            );
            BlockableAttackerDto {
                attacker_id: card_id_str(attacker),
                valid_blocker_ids: blockers
                    .iter()
                    .filter(|&&blocker| can_block(blocker, attacker))
                    .map(|&blocker| card_id_str(blocker))
                    .collect(),
                min_blockers: min as u32,
                max_blockers: (max != i32::MAX).then_some(max as u32),
                must_be_blocked: combat_util::get_lure_type(game.card(attacker)) != LureType::None,
            }
        })
        .collect();
    let block_requirements: Vec<BlockRequirementDto> = blockers
        .iter()
        .filter_map(|&blocker| {
            let targets = combat_util::compute_must_block_targets(&game, &combat, blocker);
            (!targets.is_empty()).then(|| BlockRequirementDto {
                blocker_id: card_id_str(blocker),
                attacker_ids: PromptAgent::<T>::card_ids(&targets),
            })
        })
        .collect();
    agent.send_prompt(
        PromptInput::ChooseBlockers(ChooseBlockersInput {
            attackers: attacker_options,
            available_blocker_ids: PromptAgent::<T>::card_ids(&blockers),
            block_requirements: (!block_requirements.is_empty()).then_some(block_requirements),
            error: None,
        }),
        None,
    );
    match agent.recv_action() {
        PromptOutput::ChooseBlockers(ChooseBlockersOutput::DeclareBlockers { assignments }) => {
            assignments
                .iter()
                .filter_map(
                    |BlockAssignment {
                         blocker_id,
                         attacker_id,
                     }| {
                        let b = parse_card_id(blocker_id)?;
                        let a = parse_card_id(attacker_id)?;
                        Some((b, a))
                    },
                )
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn choose_damage_assignment_order<T: Responder>(
    agent: &mut PromptAgent<T>,
    _player: PlayerId,
    attacker: CardId,
    blockers: &[CardId],
) -> Vec<CardId> {
    let attacker_id = card_id_str(attacker);
    let blocker_ids: Vec<String> = blockers.iter().map(|&b| card_id_str(b)).collect();
    let blocker_cards: Vec<CardDto> = Vec::new(); // Blocker info available from gameView
    agent.send_prompt(
        PromptInput::ChooseDamageAssignmentOrder(manabrew_protocol::prompts::choose_damage_assignment_order::ChooseDamageAssignmentOrderInput {
            attacker_id,
            blocker_ids,
            blocker_cards,
        }),
        None,
    );
    match agent.recv_action() {
        PromptOutput::ChooseDamageAssignmentOrder(
            ChooseDamageAssignmentOrderOutput::DamageAssignmentOrderDecision {
                ordered_blocker_ids,
            },
        ) => {
            let parsed: Vec<CardId> = ordered_blocker_ids
                .iter()
                .filter_map(|s| parse_card_id(s))
                .collect();
            if parsed.len() == blockers.len() {
                parsed
            } else {
                blockers.to_vec()
            }
        }
        _ => blockers.to_vec(),
    }
}

pub(super) fn choose_combat_damage_assignment<T: Responder>(
    agent: &mut PromptAgent<T>,
    _player: PlayerId,
    attacker: CardId,
    blockers_in_order: &[CardId],
    defender: Option<DefenderId>,
    total_damage: i32,
    attacker_has_deathtouch: bool,
) -> Vec<(Option<CardId>, i32)> {
    if total_damage <= 0 {
        return Vec::new();
    }
    let attacker_id = card_id_str(attacker);
    let blocker_ids: Vec<String> = blockers_in_order.iter().map(|&b| card_id_str(b)).collect();
    let defender_id = defender.map(defender_id_str);
    agent.send_prompt(
        PromptInput::ChooseCombatDamageAssignment(manabrew_protocol::prompts::choose_combat_damage_assignment::ChooseCombatDamageAssignmentInput {
            attacker_id,
            blocker_ids: blocker_ids.clone(),
            defender_id: defender_id.clone(),
            total_damage,
            attacker_has_deathtouch,
        }),
        None,
    );

    match agent.recv_action() {
        PromptOutput::ChooseCombatDamageAssignment(
            ChooseCombatDamageAssignmentOutput::CombatDamageAssignmentDecision { assignments },
        ) => assignments
            .into_iter()
            .map(|entry| {
                if defender_id
                    .as_ref()
                    .map(|d| d == &entry.assignee_id)
                    .unwrap_or(false)
                {
                    (None, entry.damage)
                } else if let Some(blocker) = parse_card_id(&entry.assignee_id) {
                    if blockers_in_order.contains(&blocker) {
                        (Some(blocker), entry.damage)
                    } else {
                        (None, 0)
                    }
                } else {
                    (None, 0)
                }
            })
            .filter(|(_, damage)| *damage > 0)
            .collect(),
        _ => fallback_combat_assignment(blockers_in_order, defender, total_damage),
    }
}

pub(super) fn pay_combat_cost<T: Responder>(
    agent: &mut PromptAgent<T>,
    _player: PlayerId,
    attacker: CardId,
    cost: i32,
    description: &str,
    mana_ability_options: &[ManaAbilityOption],
    _tappable_lands: &[CardId],
    untappable_lands: &[CardId],
    mana_pool_total: i32,
) -> CombatCostAction {
    let attacker_id = card_id_str(attacker);
    let attacker_name = agent
        .latest_view
        .as_ref()
        .and_then(|v| v.all_zone_cards().find(|c| c.id == attacker_id))
        .map(|c| c.identity.name.clone())
        .unwrap_or_default();
    let mut actions = mana_payment_actions(mana_ability_options);
    for &land in untappable_lands {
        let id = card_id_str(land);
        actions.push(PaymentAction {
            id: format!("untap:{id}"),
            kind: PaymentActionKind::UndoMana { card_id: id },
        });
    }

    agent.send_prompt(
        PromptInput::PayManaCost(
            manabrew_protocol::prompts::pay_mana_cost::PayManaCostInput {
                presentation: PromptPresentation {
                    title: attacker_name.clone(),
                    description: None,
                    text: (!description.trim().is_empty()).then(|| description.to_string()),
                    targets: Vec::new(),
                },
                card_id: attacker_id,
                card_name: attacker_name,
                mana_cost: format!("{{{cost}}}"),
                can_confirm_from_pool: mana_pool_total >= cost,
                actions,
            },
        ),
        Some(attacker),
    );
    match agent.recv_action() {
        PromptOutput::PayManaCost(PayManaCostOutput::Act { action_id }) => {
            parse_combat_cost_action(&action_id)
        }
        PromptOutput::PayManaCost(PayManaCostOutput::Pay { .. }) => CombatCostAction::Pay,
        _ => CombatCostAction::Decline,
    }
}

fn parse_combat_cost_action(action_id: &str) -> CombatCostAction {
    if let Some(rest) = action_id.strip_prefix("tap:") {
        let tap = parse_tap_action_id(rest);
        return match parse_card_id(tap.card_id) {
            Some(card_id) => CombatCostAction::TapLand {
                card_id,
                mana_ability_index: tap.ability_index,
                express_choice: parse_express_mana_choice(tap.color),
            },
            None => CombatCostAction::Decline,
        };
    }
    if let Some(id) = action_id.strip_prefix("untap:") {
        return parse_card_id(id)
            .map(CombatCostAction::UntapLand)
            .unwrap_or(CombatCostAction::Decline);
    }
    CombatCostAction::Decline
}

pub(super) fn exert_attackers<T: Responder>(
    agent: &mut PromptAgent<T>,
    _player: PlayerId,
    attackers: &[CardId],
) -> Vec<CardId> {
    super::targeting::choose_board_targets_multi(
        agent,
        attackers,
        TargetingIntent::Tap,
        "Exert",
        None,
    )
}

pub(super) fn enlist_attackers<T: Responder>(
    agent: &mut PromptAgent<T>,
    _player: PlayerId,
    attackers: &[CardId],
) -> Vec<CardId> {
    super::targeting::choose_board_targets_multi(
        agent,
        attackers,
        TargetingIntent::Tap,
        "Enlist",
        None,
    )
}
