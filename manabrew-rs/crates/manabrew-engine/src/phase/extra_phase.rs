//! ExtraPhase — stores an extra phase inserted into the turn.
//!
//! Mirrors Java's `ExtraPhase.java`.

use forge_foundation::PhaseType;
use serde::{Deserialize, Serialize};

use crate::trigger::handler::DelayedTrigger;

/// An extra phase entry — tracks what phase to insert and any delayed triggers.
/// Mirrors Java's `ExtraPhase` class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraPhase {
    phase: PhaseType,
    #[serde(skip)]
    delayed_triggers: Vec<DelayedTrigger>,
}

impl ExtraPhase {
    pub fn new(phase: PhaseType) -> Self {
        ExtraPhase {
            phase,
            delayed_triggers: Vec::new(),
        }
    }

    pub fn get_phase(&self) -> PhaseType {
        self.phase
    }

    pub fn add_trigger(&mut self, del_trigger: DelayedTrigger) {
        self.delayed_triggers.push(del_trigger);
    }

    pub fn get_delayed_triggers(&self) -> &[DelayedTrigger] {
        &self.delayed_triggers
    }
}
