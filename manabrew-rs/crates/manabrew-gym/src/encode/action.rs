use manabrew_engine::combat::DefenderId;
use manabrew_engine::ids::CardId;

use crate::decision::{Action, ActionError, DecisionKind, PriorityOption, TargetOption};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    Priority(PriorityOption),
    LandOrSpell(usize),
    Ability {
        index: usize,
        source: Option<CardId>,
    },
    Attack {
        slot: usize,
        attacker: CardId,
        defender: usize,
        target: DefenderId,
    },
    Block {
        slot: usize,
        blocker: CardId,
        attacker: usize,
        attacker_id: CardId,
    },
    Target(TargetOption),
    Card(CardId),
    Mode(usize),
    Confirm(bool),
}

pub fn candidates(kind: &DecisionKind) -> Vec<Candidate> {
    match kind {
        DecisionKind::Priority { options } => {
            options.iter().map(|&o| Candidate::Priority(o)).collect()
        }
        DecisionKind::LandOrSpell => (0..3).map(Candidate::LandOrSpell).collect(),
        DecisionKind::AbilityToPlay { abilities } => abilities
            .iter()
            .enumerate()
            .map(|(index, a)| Candidate::Ability {
                index,
                source: a.source,
            })
            .collect(),
        DecisionKind::Attackers {
            attackers,
            defenders,
            legal,
        } => attackers
            .iter()
            .zip(legal)
            .enumerate()
            .flat_map(|(slot, (&attacker, allowed))| {
                allowed.iter().map(move |&defender| Candidate::Attack {
                    slot,
                    attacker,
                    defender,
                    target: defenders[defender],
                })
            })
            .collect(),
        DecisionKind::Blockers {
            attackers,
            blockers,
            legal,
            ..
        } => blockers
            .iter()
            .zip(legal)
            .enumerate()
            .flat_map(|(slot, (&blocker, allowed))| {
                allowed.iter().map(move |&attacker| Candidate::Block {
                    slot,
                    blocker,
                    attacker,
                    attacker_id: attackers[attacker],
                })
            })
            .collect(),
        DecisionKind::Target { candidates, .. } => {
            candidates.iter().map(|&t| Candidate::Target(t)).collect()
        }
        DecisionKind::Cards { cards, .. } => cards.iter().map(|&c| Candidate::Card(c)).collect(),
        DecisionKind::Modes { descriptions, .. } => {
            (0..descriptions.len()).map(Candidate::Mode).collect()
        }
        DecisionKind::Confirm { .. } | DecisionKind::Mulligan { .. } => {
            vec![Candidate::Confirm(true), Candidate::Confirm(false)]
        }
    }
}

pub fn action_from_picks(kind: &DecisionKind, picks: &[usize]) -> Result<Action, ActionError> {
    let action = match kind {
        DecisionKind::Priority { .. }
        | DecisionKind::LandOrSpell
        | DecisionKind::AbilityToPlay { .. }
        | DecisionKind::Target { .. } => match picks {
            [pick] => Action::Choose(*pick),
            _ => return Err(ActionError::WrongShape),
        },
        DecisionKind::Confirm { .. } | DecisionKind::Mulligan { .. } => match picks {
            [pick] if *pick < 2 => Action::Confirm(*pick == 0),
            [pick] => {
                return Err(ActionError::OutOfRange {
                    index: *pick,
                    len: 2,
                })
            }
            _ => return Err(ActionError::WrongShape),
        },
        DecisionKind::Cards { .. } | DecisionKind::Modes { .. } => Action::Select(picks.to_vec()),
        DecisionKind::Attackers { legal, .. } | DecisionKind::Blockers { legal, .. } => {
            let flat = candidates(kind);
            let mut slots = vec![None; legal.len()];
            for &pick in picks {
                let (slot, choice) = match flat.get(pick) {
                    Some(Candidate::Attack { slot, defender, .. }) => (*slot, *defender),
                    Some(Candidate::Block { slot, attacker, .. }) => (*slot, *attacker),
                    _ => {
                        return Err(ActionError::OutOfRange {
                            index: pick,
                            len: flat.len(),
                        })
                    }
                };
                if slots[slot].replace(choice).is_some() {
                    return Err(ActionError::Duplicate(pick));
                }
            }
            Action::Assign(slots)
        }
    };
    kind.validate(&action)?;
    Ok(action)
}
