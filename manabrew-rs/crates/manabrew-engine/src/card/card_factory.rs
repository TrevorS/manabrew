use std::sync::Arc;

use forge_carddb::CardRules;
use forge_foundation::ZoneType;

use super::card_assembly;
use super::Card;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;

/// Build a `Card` from card rules using the 3-phase card assembly
/// pipeline.
pub(crate) fn build_from_rules(rules: &CardRules, owner: PlayerId) -> Card {
    // Phase 1: Parse raw text into components.
    let mut components = card_assembly::parse_card_components(&rules.main_part);

    // Phase 2: Synthesize derived triggers/keywords (Magecraft, Exert, etc.).
    // Pass 0 as existing trigger count — keyword-generated triggers are added
    // by the constructor in Phase 3, so we don't know the count yet.
    card_assembly::synthesize_derived(&mut components, 0);

    // Phase 3: Assemble into Card.
    card_assembly::assemble_card(rules, owner, components)
}

/// Compatibility entrypoint for parity with Java `CardFactory`.
pub fn from_rules(rules: &CardRules, owner: PlayerId) -> Card {
    build_from_rules(rules, owner)
}

/// Java-parity helper for `CardFactory.copySpellAbilityAndPossiblyHost`.
///
/// The full host-card cloning path is not yet present in the Rust engine, but
/// this preserves current copy semantics and centralizes them in the card
/// module so effects can call one canonical implementation.
pub fn copy_spell_ability(target_sa: &SpellAbility, controller: PlayerId) -> SpellAbility {
    let mut copy = target_sa.clone();
    let mut node = Some(&mut copy);
    while let Some(sa) = node {
        sa.id = crate::spellability::next_spell_ability_id();
        node = sa.sub_ability.as_deref_mut();
    }
    copy.set_activating_player(controller);
    copy.is_copy = true;
    // Copied spells/abilities are not re-cast and should not require paying costs.
    copy.pay_costs = None;
    copy
}

fn copy_spell_host(
    game: &mut GameState,
    source_sa: &SpellAbility,
    target_sa: &SpellAbility,
    controller: PlayerId,
) -> Option<CardId> {
    let original = target_sa.source?;
    let mut copy = crate::ability::effects::copy_permanent_effect::get_proto_type(
        source_sa,
        game.card(original),
        controller,
    );
    copy.copied_permanent = Some(original);
    let copy_id = game.create_card(copy);
    game.card_mut(copy_id).zone = ZoneType::Stack;
    game.add_card_to_zone(ZoneType::Stack, controller, copy_id);
    game.assign_zone_timestamp(copy_id);
    if crate::parsing::raw_has_key(&source_sa.ability_text, "RememberNewCard") {
        if let Some(source) = source_sa.source {
            game.card_mut(source).add_remembered_card(copy_id);
        }
    }
    Some(copy_id)
}

pub fn copy_spell_ability_and_possibly_host(
    game: &mut GameState,
    source_sa: &SpellAbility,
    target_sa: &SpellAbility,
    controller: PlayerId,
) -> SpellAbility {
    let host = if target_sa.is_spell
        && !crate::parsing::raw_has_key(&source_sa.ability_text, "UseOriginalHost")
    {
        copy_spell_host(game, source_sa, target_sa, controller)
    } else {
        None
    };
    let mut copy = copy_spell_ability(target_sa, controller);
    if let Some(host) = host {
        copy.set_host_card_id(host);
    }
    copy
}

pub fn is_copied_spell_host(game: &GameState, sa: &SpellAbility) -> bool {
    sa.is_copy
        && sa.is_spell
        && sa
            .source
            .is_some_and(|host| game.card(host).copied_permanent.is_some())
}

/// Java parity helper for `SpellAbility.cantBeCopied()` checks.
pub fn spell_ability_cant_be_copied(cards: &[Arc<Card>], sa: &SpellAbility) -> bool {
    sa.source
        .map(|source| {
            crate::staticability::static_ability_cant_be_copied::cant_be_copied(
                cards,
                &cards[source.index()],
            )
        })
        .unwrap_or(false)
}
