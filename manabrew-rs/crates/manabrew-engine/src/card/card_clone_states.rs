//! Clone-state container (Java parity: `CardCloneStates`).

use crate::card::Card;
use crate::ids::CardId;
use crate::spellability::SpellAbility;

#[derive(Debug, Clone)]
pub struct CardCloneStates {
    origin: CardId,
    source: Box<SpellAbility>,
    state: Card,
}

impl CardCloneStates {
    pub fn new(origin: CardId, source: &SpellAbility, state: Card) -> Self {
        Self {
            origin,
            source: Box::new(source.clone()),
            state,
        }
    }

    pub fn get_origin(&self) -> CardId {
        self.origin
    }

    pub fn get_source(&self) -> &SpellAbility {
        &self.source
    }

    pub fn get_host(&self) -> Option<CardId> {
        self.source.source
    }

    pub fn state(&self) -> &Card {
        &self.state
    }

    pub fn into_state(self) -> Card {
        self.state
    }
}
