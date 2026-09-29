//! CostPaymentStack — tracks cost payments for trigger purposes.
//!
//! Mirrors Java's `CostPaymentStack.java`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::cost::CostPart;
use crate::spellability::SpellAbility;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostPaymentStack {
    stack: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub cost: CostPart,
    pub ability: Arc<SpellAbility>,
}

impl CostPaymentStack {
    pub fn new() -> Self {
        CostPaymentStack { stack: Vec::new() }
    }

    pub fn push(&mut self, cost: CostPart, ability: Arc<SpellAbility>) {
        self.stack.push(Entry { cost, ability });
    }

    pub fn pop(&mut self) -> Option<Entry> {
        self.stack.pop()
    }

    pub fn peek(&self) -> Option<&Entry> {
        self.stack.last()
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }

    pub fn size(&self) -> usize {
        self.stack.len()
    }

    pub fn truncate(&mut self, len: usize) {
        self.stack.truncate(len);
    }

    pub fn iterator(&self) -> impl Iterator<Item = &Entry> {
        self.stack.iter()
    }
}
