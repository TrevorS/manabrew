use std::fmt;

use manabrew_engine::agent::PlayOption;
use manabrew_engine::combat::DefenderId;
use manabrew_engine::ids::{CardId, PlayerId};
use manabrew_engine::player::actions::{AbilityRef, PlayerAction};

use crate::encode::Observation;

#[derive(Debug, Clone)]
pub struct Decision {
    pub player: PlayerId,
    pub turn: u32,
    pub kind: DecisionKind,
    pub observation: Option<Box<Observation>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionKind {
    Priority {
        options: Vec<PriorityOption>,
    },
    LandOrSpell,
    AbilityToPlay {
        abilities: Vec<AbilityOption>,
    },
    Attackers {
        attackers: Vec<CardId>,
        defenders: Vec<DefenderId>,
        legal: Vec<Vec<usize>>,
    },
    Blockers {
        attackers: Vec<CardId>,
        blockers: Vec<CardId>,
        legal: Vec<Vec<usize>>,
        max: Option<usize>,
    },
    Target {
        candidates: Vec<TargetOption>,
        source: Option<CardId>,
    },
    Cards {
        purpose: CardPurpose,
        cards: Vec<CardId>,
        min: usize,
        max: usize,
        source: Option<CardId>,
    },
    Modes {
        descriptions: Vec<String>,
        min: usize,
        max: usize,
        source: Option<CardId>,
    },
    Confirm {
        purpose: ConfirmPurpose,
        text: String,
        source: Option<CardId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityOption {
    Pass,
    Play(PlayOption),
    Activate(AbilityRef),
}

impl PriorityOption {
    pub fn to_player_action(self) -> PlayerAction {
        match self {
            PriorityOption::Pass => PlayerAction::PassPriority,
            PriorityOption::Play(play) => PlayerAction::CastSpell(play),
            PriorityOption::Activate(ability) => PlayerAction::ActivateAbility(ability),
        }
    }
}

pub const LAND_OR_SPELL: [Option<bool>; 3] = [Some(true), Some(false), None];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityOption {
    pub source: Option<CardId>,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOption {
    Player(PlayerId),
    Card(CardId),
    Stack(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPurpose {
    Effect,
    Sacrifice,
    Discard,
    Target,
    Dig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmPurpose {
    OptionalTrigger,
    Action(Option<String>),
    Replacement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Choose(usize),
    Select(Vec<usize>),
    Assign(Vec<Option<usize>>),
    Confirm(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    WrongShape,
    OutOfRange {
        index: usize,
        len: usize,
    },
    Duplicate(usize),
    Count {
        picked: usize,
        min: usize,
        max: usize,
    },
    Illegal {
        slot: usize,
        index: usize,
    },
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::WrongShape => write!(f, "action does not match the decision kind"),
            ActionError::OutOfRange { index, len } => {
                write!(f, "index {index} out of range for {len} options")
            }
            ActionError::Duplicate(index) => write!(f, "index {index} picked twice"),
            ActionError::Count { picked, min, max } => {
                write!(f, "picked {picked}, expected {min}..={max}")
            }
            ActionError::Illegal { slot, index } => {
                write!(f, "slot {slot} cannot take option {index}")
            }
        }
    }
}

impl std::error::Error for ActionError {}

impl DecisionKind {
    pub const COUNT: usize = 9;

    pub fn id(&self) -> usize {
        match self {
            DecisionKind::Priority { .. } => 0,
            DecisionKind::LandOrSpell => 1,
            DecisionKind::AbilityToPlay { .. } => 2,
            DecisionKind::Attackers { .. } => 3,
            DecisionKind::Blockers { .. } => 4,
            DecisionKind::Target { .. } => 5,
            DecisionKind::Cards { .. } => 6,
            DecisionKind::Modes { .. } => 7,
            DecisionKind::Confirm { .. } => 8,
        }
    }

    pub fn validate(&self, action: &Action) -> Result<(), ActionError> {
        match (self, action) {
            (DecisionKind::Priority { options }, Action::Choose(i)) => in_range(*i, options.len()),
            (DecisionKind::LandOrSpell, Action::Choose(i)) => in_range(*i, LAND_OR_SPELL.len()),
            (DecisionKind::AbilityToPlay { abilities }, Action::Choose(i)) => {
                in_range(*i, abilities.len())
            }
            (DecisionKind::Target { candidates, .. }, Action::Choose(i)) => {
                in_range(*i, candidates.len())
            }
            (
                DecisionKind::Cards {
                    cards, min, max, ..
                },
                Action::Select(picks),
            ) => validate_set(picks, cards.len(), *min, *max),
            (
                DecisionKind::Modes {
                    descriptions,
                    min,
                    max,
                    ..
                },
                Action::Select(picks),
            ) => validate_set(picks, descriptions.len(), *min, *max),
            (DecisionKind::Attackers { legal, .. }, Action::Assign(slots)) => {
                validate_assign(slots, legal, None)
            }
            (DecisionKind::Blockers { legal, max, .. }, Action::Assign(slots)) => {
                validate_assign(slots, legal, *max)
            }
            (DecisionKind::Confirm { .. }, Action::Confirm(_)) => Ok(()),
            _ => Err(ActionError::WrongShape),
        }
    }
}

fn in_range(index: usize, len: usize) -> Result<(), ActionError> {
    if index < len {
        Ok(())
    } else {
        Err(ActionError::OutOfRange { index, len })
    }
}

fn validate_set(picks: &[usize], len: usize, min: usize, max: usize) -> Result<(), ActionError> {
    if picks.len() < min || picks.len() > max {
        return Err(ActionError::Count {
            picked: picks.len(),
            min,
            max,
        });
    }
    for (n, &index) in picks.iter().enumerate() {
        in_range(index, len)?;
        if picks[..n].contains(&index) {
            return Err(ActionError::Duplicate(index));
        }
    }
    Ok(())
}

fn validate_assign(
    slots: &[Option<usize>],
    legal: &[Vec<usize>],
    max: Option<usize>,
) -> Result<(), ActionError> {
    if slots.len() != legal.len() {
        return Err(ActionError::WrongShape);
    }
    for (slot, (choice, allowed)) in slots.iter().zip(legal).enumerate() {
        if let Some(index) = *choice {
            if !allowed.contains(&index) {
                return Err(ActionError::Illegal { slot, index });
            }
        }
    }
    let picked = slots.iter().flatten().count();
    match max {
        Some(max) if picked > max => Err(ActionError::Count {
            picked,
            min: 0,
            max,
        }),
        _ => Ok(()),
    }
}
