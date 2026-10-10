//! Card copy helpers (Java parity: `CardCopyService`).

use crate::card::Card;
use crate::ids::{CardId, PlayerId};

/// Copy a card, optionally assigning a new id/owner.
pub fn copy_card(
    copy_from: &Card,
    assign_new_id: bool,
    owner: Option<PlayerId>,
    new_id: Option<CardId>,
) -> Card {
    let mut out = copy_from.clone();
    if assign_new_id {
        if let Some(id) = new_id {
            out.id = id;
        }
    }
    if let Some(new_owner) = owner {
        out.owner = new_owner;
    }
    out
}

/// Java parity helper used by multiple copy paths.
pub fn copy_stats(
    input: &Card,
    new_owner: Option<PlayerId>,
    assign_new_id: bool,
    new_id: Option<CardId>,
) -> Card {
    copy_card(input, assign_new_id, new_owner, new_id)
}

/// Copy copiable characteristics from one card into another.
pub fn copy_copiable_characteristics(copy_from: &Card, to: &mut Card) {
    to.card_name = copy_from.card_name.clone();
    to.oracle_text = copy_from.oracle_text.clone();
    to.state_type_line = copiable_type_line(copy_from);
    to.update_type_cache();
    to.mana_cost = copy_from.mana_cost.clone();
    to.color = copy_from
        .animate_state
        .as_ref()
        .map(|state| state.original_color)
        .unwrap_or(copy_from.color);
    to.base_power = copy_from
        .animate_state
        .as_ref()
        .map(|state| state.original_base_power)
        .or(copy_from.changed_base_power)
        .unwrap_or(copy_from.base_power);
    to.base_toughness = copy_from
        .animate_state
        .as_ref()
        .map(|state| state.original_base_toughness)
        .or(copy_from.changed_base_toughness)
        .unwrap_or(copy_from.base_toughness);
    to.initial_loyalty = copy_from.initial_loyalty.clone();
    to.keywords = copy_from.keywords.clone();
    to.abilities = copy_from.abilities.clone();
    to.triggers = copy_from.triggers.clone();
    for trigger in &mut to.triggers {
        trigger.bind_host_card_id(to.id);
        trigger.original_host.get_or_insert(copy_from.id);
    }
    to.svars = copy_from.svars.clone();
    if copy_from.other_part.is_some() {
        to.other_part = copy_from.other_part.clone();
        to.is_transformed = copy_from.is_transformed;
    }
    to.parsed_svar_cache.clear();
    to.refresh_action_specs();
}

pub(crate) fn copiable_type_line(copy_from: &Card) -> forge_foundation::CardTypeLine {
    copy_from.state_type_line.clone()
}

/// Return a Last Known Information snapshot of `card` — its state frozen at
/// the point of the call. Mirrors Java `CardCopyService.getLKICopy(Card)`.
///
/// The snapshot locks in effective power/toughness (as of the current layer
/// stack) as new base values, captures current counters, zone, tapped/phased
/// status, and the remembered / imprinted / chosen-* buckets. Consumers use
/// LKI copies to answer "what was this creature when it died?" style
/// queries without being disturbed by subsequent layer recomputation.
pub fn get_lki_copy(card: &Card) -> Card {
    let mut lki = card.clone();

    // Lock in effective P/T so further modifiers don't shift it.
    let current_power = card.power();
    let current_toughness = card.toughness();
    lki.base_power = Some(current_power);
    lki.base_toughness = Some(current_toughness);
    lki.power_modifier = 0;
    lki.toughness_modifier = 0;
    lki.static_power_modifier = 0;
    lki.static_toughness_modifier = 0;
    lki.perpetual_power_modifier = 0;
    lki.perpetual_toughness_modifier = 0;
    lki.pt_boosts.clear();
    lki.lki_power = Some(current_power);
    lki.lki_toughness = Some(current_toughness);

    // Snapshot counters. Mirrors Java `newCopy.setCounters(Maps.newHashMap(copyFrom.getCounters()))`.
    lki.lki_counters = Some(card.counters.clone());

    lki
}
