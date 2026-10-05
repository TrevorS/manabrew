//! Blight as a cost — put -1/-1 counters on creatures you control.
//! Mirrors Java's `CostBlight` which extends `CostPutCounter`.

use crate::card::CounterType;
use crate::game::GameState;
use crate::ids::CardId;

pub fn to_string(part: &super::CostPart) -> String {
    match part {
        super::CostPart::Blight(amount) => format!("Blight {amount}"),
        _ => String::new(),
    }
}

/// Execute blight payment for selected creatures.
/// Puts a -1/-1 counter on each chosen creature.
pub fn pay_as_decided_cards(
    game: &mut GameState,
    player: crate::ids::PlayerId,
    cards: &[CardId],
    amount: i32,
) -> bool {
    if let Some(&card) = cards.first() {
        crate::ability::effects::effect_context::add_counter_with_context(
            game,
            None,
            None,
            card,
            &CounterType::M1M1,
            amount,
            crate::event::RunParams {
                source_player: Some(player),
                ..Default::default()
            },
            false,
        );
    }
    true
}

/// Refund blight payment — remove the -1/-1 counters.
pub fn refund(game: &mut GameState, cards: &[CardId]) {
    for &cid in cards {
        game.card_mut(cid).remove_counter(&CounterType::M1M1, 1);
    }
}

pub fn can_pay(
    game: &crate::game::GameState,
    _source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::Blight(_) = part else {
        return false;
    };
    let has_creature = game
        .cards_in_zone(forge_foundation::ZoneType::Battlefield, player)
        .iter()
        .any(|&cid| {
            let c = game.card(cid);
            c.is_creature()
                && !c.phased_out
                && !crate::staticability::static_ability_cant_put_counter::any_cant_put_counter_on_card(
                    game,
                    c,
                    &CounterType::M1M1,
                )
        });
    has_creature
}
