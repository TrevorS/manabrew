//! Put counters on permanents as a cost. Mirrors Java's `CostPutCounter`.
//!
//! Java's `CostPutCounter` extends `CostPartWithList` and manages counter
//! placement on source or target permanents. It also handles ETB replacement
//! effects where counters are placed as the card enters the battlefield.

use crate::card::CounterType;
use crate::game::GameState;
use crate::ids::CardId;

/// Add counters to the source.
/// Mirrors Java's `CostPutCounter.doPayment()`.
pub fn pay_as_decided(
    game: &mut GameState,
    player: crate::ids::PlayerId,
    source: CardId,
    amount: i32,
    counter_type: &CounterType,
) -> bool {
    crate::ability::effects::effect_context::add_counter_with_context(
        game,
        None,
        None,
        source,
        counter_type,
        amount,
        crate::event::RunParams {
            source_player: Some(player),
            ..Default::default()
        },
        false,
    );
    // TODO: Fire counter placement triggers via GameEntityCounterTable
    // Java's CostPutCounter.triggerCounterPutAll() handles this
    true
}

/// Refund by removing the placed counters.
/// Mirrors Java's `CostPutCounter.refund()`.
pub fn refund(game: &mut GameState, source: CardId, amount: i32, counter_type: &CounterType) {
    game.card_mut(source).remove_counter(counter_type, amount);
}

pub const HASH_LKI: &str = "CounterPut";
pub const HASH_CARDS: &str = "CounterPutCards";

pub fn payment_order(part: &super::CostPart) -> i32 {
    part.payment_order()
}

/// Java `CostPart.payCostFromSource`: only CARDNAME and NICKNAME mean the source itself;
/// anything else is a valid string searched over the battlefield.
fn pays_from_source(type_filter: &str) -> bool {
    type_filter.eq_ignore_ascii_case("CARDNAME") || type_filter.eq_ignore_ascii_case("NICKNAME")
}

/// The permanents this cost may put its counters on, in Java's order.
/// Mirrors the `CardLists.getValidCards` call in `CostPutCounter.canPay:154`.
fn candidates(
    game: &crate::game::GameState,
    source: CardId,
    ability: Option<&crate::spellability::SpellAbility>,
    type_filter: &str,
) -> Vec<CardId> {
    let source_card = game.card(source);
    let selector = crate::parsing::cached_compiled_selector(type_filter);
    let targeted: Vec<CardId> = ability
        .map(|sa| sa.target_chosen.all_target_cards())
        .unwrap_or_default();
    let targeted_players: Vec<crate::ids::PlayerId> = ability
        .map(|sa| sa.target_chosen.all_target_players())
        .unwrap_or_default();
    let mut context = crate::card::valid_filter::MatchContext::from_source(source_card)
        .with_targets(&targeted, &targeted_players);
    context.game = Some(game);
    context.spell_ability = ability;
    game.players
        .iter()
        .flat_map(|p| {
            game.cards_in_zone(forge_foundation::ZoneType::Battlefield, p.id)
                .to_vec()
        })
        .filter(|&cid| {
            crate::card::valid_filter::matches_valid_card_selector_with_context(
                &selector,
                game.card(cid),
                context,
            )
        })
        .collect()
}

/// Which permanent this cost puts its counters on: the source for CARDNAME, otherwise
/// the first battlefield permanent matching the valid string, in Java's order.
pub fn counter_target(
    game: &crate::game::GameState,
    source: CardId,
    ability: Option<&crate::spellability::SpellAbility>,
    type_filter: &str,
) -> Option<CardId> {
    if pays_from_source(type_filter) {
        return Some(source);
    }
    candidates(game, source, ability, type_filter)
        .into_iter()
        .next()
}

pub fn can_pay(
    game: &crate::game::GameState,
    _available_mana: &crate::mana::ManaPool,
    source: crate::ids::CardId,
    _player: crate::ids::PlayerId,
    ability: Option<&crate::spellability::SpellAbility>,
    part: &super::CostPart,
) -> bool {
    let super::CostPart::AddCounter { type_filter, .. } = part else {
        return false;
    };
    if pays_from_source(type_filter) {
        return game.card(source).zone == forge_foundation::ZoneType::Battlefield;
    }
    !candidates(game, source, ability, type_filter).is_empty()
}

pub fn pay_with_decision(
    game: &mut GameState,
    player: crate::ids::PlayerId,
    source: CardId,
    part: &super::CostPart,
    _decision: &crate::cost::payment_decision::PaymentDecision,
) -> bool {
    let super::CostPart::AddCounter {
        amount,
        counter_type,
        type_filter,
    } = part
    else {
        return false;
    };
    let resolved = amount.resolve(game, source, player);
    if pays_from_source(type_filter) {
        return pay_as_decided(game, player, source, resolved, counter_type);
    }
    let Some(target) = candidates(game, source, None, type_filter)
        .into_iter()
        .next()
    else {
        return false;
    };
    pay_as_decided(game, player, target, resolved, counter_type)
}

pub fn reset_lists() {}
