//! Put counters on the paying player as a cost. Mirrors Java's `CostPutCounterYou`.

use crate::agent::PlayerAgent;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerHandler;

pub fn can_pay(
    game: &GameState,
    _available_mana: &crate::mana::ManaPool,
    _source: CardId,
    player: PlayerId,
    _ability: Option<&SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::PutCounterYou { counter_type, .. } = part else {
        return false;
    };
    !crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_player(
        &game.cards,
        player,
        counter_type,
    )
}

pub fn pay_as_decided(
    game: &mut GameState,
    trigger_handler: Option<&mut TriggerHandler>,
    agents: Option<&mut [Box<dyn PlayerAgent>]>,
    payer: PlayerId,
    source: CardId,
    ability: Option<&SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::PutCounterYou {
        amount,
        counter_type,
    } = part
    else {
        return false;
    };
    let c = amount.resolve_for_sa(game, source, payer, ability);
    let mut table = crate::game_entity_counter_table::GameEntityCounterTable::default();
    table.put(
        Some(payer),
        crate::agent::GameEntity::Player(payer),
        counter_type.clone(),
        c,
    );
    table.replace_counter_effect(
        game,
        trigger_handler,
        agents,
        ability,
        false,
        Default::default(),
    );
    true
}

pub fn pay_with_decision(
    game: &mut GameState,
    player: PlayerId,
    source: CardId,
    part: &super::CostPart,
    _decision: &crate::cost::payment_decision::PaymentDecision,
) -> bool {
    pay_as_decided(game, None, None, player, source, None, part)
}
