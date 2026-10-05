//! Parity shim for Java `CostRemoveCounter`.

use crate::card::CounterType;
use forge_foundation::lang;

pub fn to_string(part: &super::CostPart) -> String {
    let super::CostPart::SubCounter {
        amount,
        counter_type,
        type_filter,
        description,
    } = part
    else {
        return String::new();
    };
    let amount_text = amount.to_string();
    let from_source = super::cost_part::type_is_source(type_filter);
    let name = counter_type.get_name().to_lowercase();
    if *counter_type == CounterType::Loyalty && from_source {
        return format!("-{amount_text}");
    }
    let mut sb = String::from("Remove ");
    if amount_text == "X" {
        sb.push_str(&format!("any number of {name} counters"));
    } else if amount_text == "All" {
        sb.push_str(&format!("all {name} counters"));
    } else {
        sb.push_str(&lang::noun_with_numeral_except_one(
            &amount_text,
            &format!("{name} counter"),
        ));
    }
    sb.push_str(" from ");
    if from_source {
        sb.push_str(type_filter);
    } else {
        sb.push_str(description.as_deref().unwrap_or(type_filter));
    }
    sb
}

pub fn pay_as_decided(
    game: &mut crate::game::GameState,
    source: crate::ids::CardId,
    amount: i32,
    counter_type: &crate::card::CounterType,
) -> bool {
    crate::cost::cost_sub_counter::pay_as_decided(game, source, amount, counter_type)
}

pub fn refund(
    game: &mut crate::game::GameState,
    source: crate::ids::CardId,
    amount: i32,
    counter_type: &crate::card::CounterType,
) {
    crate::cost::cost_sub_counter::refund(game, source, amount, counter_type);
}

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    player: crate::ids::PlayerId,
    _ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::SubCounter {
        amount,
        counter_type,
        type_filter,
        ..
    } = part
    else {
        return false;
    };
    crate::cost::cost_sub_counter::can_pay_for_player(
        game,
        source,
        player,
        amount.resolve(game, source, player),
        counter_type,
        type_filter,
    )
}

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}
