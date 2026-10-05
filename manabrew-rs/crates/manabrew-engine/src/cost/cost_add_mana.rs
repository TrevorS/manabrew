//! Add mana to pool as a cost. Mirrors Java's `CostAddMana`.

use forge_foundation::mana::ManaAtom;

use crate::ids::{CardId, PlayerId};
use crate::mana::{Mana, ManaPool};

pub fn to_string(part: &super::CostPart) -> String {
    match part {
        super::CostPart::AddMana { amount, mana_type } => format!(
            "Add {}",
            format!("{{{mana_type}}}").repeat(amount.as_literal().unwrap_or(0).max(0) as usize)
        ),
        _ => String::new(),
    }
}

/// Pay by adding mana to the player's pool.
/// Mirrors Java's `CostAddMana.payAsDecided()`.
pub fn pay_as_decided(
    pool: &mut ManaPool,
    source: CardId,
    _player: PlayerId,
    amount: i32,
    mana_type: &str,
) -> bool {
    let atom = match mana_type.to_uppercase().as_str() {
        "W" | "WHITE" => ManaAtom::WHITE,
        "U" | "BLUE" => ManaAtom::BLUE,
        "B" | "BLACK" => ManaAtom::BLACK,
        "R" | "RED" => ManaAtom::RED,
        "G" | "GREEN" => ManaAtom::GREEN,
        "C" | "COLORLESS" => ManaAtom::COLORLESS,
        _ => ManaAtom::COLORLESS,
    };
    for _ in 0..amount {
        let mut m = Mana::simple(atom);
        m.source_card = Some(source);
        pool.add_mana(m);
    }
    true
}

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}

pub fn can_pay(
    _game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    _source: crate::ids::CardId,
    _player: crate::ids::PlayerId,
    _ability: Option<&crate::spellability::SpellAbility>,
    _part: &super::CostPart,
) -> bool {
    true
}
