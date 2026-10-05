//! Deal damage to self as a cost. Mirrors Java's `CostDamage`.

use crate::game::GameState;
use crate::ids::PlayerId;

pub fn to_string(part: &super::CostPart) -> String {
    match part {
        super::CostPart::DamageYou(amount) => format!("Deal {amount} damage to you"),
        _ => String::new(),
    }
}

/// Pay by dealing damage to the player.
/// Mirrors Java's `CostDamage.payAsDecided()` which creates a CardDamageMap
/// and calls `game.getAction().dealDamage()`.
/// NOTE: Trigger firing (DamageDone) must be handled by the caller.
pub fn pay_as_decided(game: &mut GameState, player: PlayerId, amount: i32) -> bool {
    game.deal_damage_to_player(player, amount);
    amount > 0
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
