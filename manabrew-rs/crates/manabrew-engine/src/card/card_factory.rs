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

/// Java `CardFactory.getCloneStates`: the copiable values of `input` (CR 707.2).
pub fn get_clone_states(input: &Card, new_owner: PlayerId, cause: &SpellAbility) -> Card {
    let mut out = Card::new(
        CardId(0),
        if input.face_down {
            String::new()
        } else {
            input.card_name.clone()
        },
        new_owner,
        super::card_copy_service::copiable_type_line(input),
        input.mana_cost.clone(),
        input
            .animate_state
            .as_ref()
            .map_or(input.color, |state| state.original_color),
        input
            .animate_state
            .as_ref()
            .map(|state| state.original_base_power)
            .or(input.changed_base_power)
            .unwrap_or(input.base_power),
        input
            .animate_state
            .as_ref()
            .map(|state| state.original_base_toughness)
            .or(input.changed_base_toughness)
            .unwrap_or(input.base_toughness),
        input.copiable_keywords().as_string_list(),
        input.abilities.clone(),
    );
    if input.face_down {
        out.activated_abilities
            .retain(|ability| !ability.is_turn_face_up());
        out.base_ability_count = out.activated_abilities.len();
    }
    out.oracle_text = input.oracle_text.clone();
    out.set_triggers(input.copiable_triggers());
    out.set_svars_map(input.svars.clone());
    out.set_static_abilities(input.copiable_static_abilities());
    out.set_replacement_effects(input.copiable_replacement_effects());
    out.initial_loyalty = input.initial_loyalty.clone();
    out.set_code = input.set_code.clone();
    let copies_other_state =
        input
            .other_part
            .as_ref()
            .is_some_and(|other| match other.state_name {
                forge_foundation::CardStateName::Secondary
                | forge_foundation::CardStateName::PreparedSpell => true,
                forge_foundation::CardStateName::Backside => {
                    !other.is_modal
                        && matches!(
                            cause.api,
                            Some(
                                crate::ability::api_type::ApiType::CopyPermanent
                                    | crate::ability::api_type::ApiType::CopySpellAbility
                                    | crate::ability::api_type::ApiType::ReplaceToken
                            )
                        )
                }
                _ => false,
            });
    if copies_other_state {
        out.other_part = input.other_part.clone();
        out.is_transformed = input.is_transformed;
    }
    out
}

/// Java-parity helper for `CardFactory.copySpellAbilityAndPossiblyHost`.
///
/// The full host-card cloning path is not yet present in the Rust engine, but
/// this preserves current copy semantics and centralizes them in the card
/// module so effects can call one canonical implementation.
pub fn copy_spell_ability(
    game: &mut GameState,
    target_sa: &SpellAbility,
    controller: PlayerId,
) -> SpellAbility {
    let mut copy = target_sa.clone();
    let mut node = Some(&mut copy);
    while let Some(sa) = node {
        sa.id = game.stack.next_spell_ability_id();
        node = sa.sub_ability.as_deref_mut();
    }
    copy.set_activating_player(controller);
    copy.is_copy = true;
    if !target_sa.is_trigger {
        copy.pay_costs = None;
    }
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
    let mut copy = copy_spell_ability(game, target_sa, controller);
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
pub fn spell_ability_cant_be_copied(game: &GameState, sa: &SpellAbility) -> bool {
    sa.source
        .map(|source| {
            crate::staticability::static_ability_cant_be_copied::cant_be_copied(
                game,
                game.card(source),
            )
        })
        .unwrap_or(false)
}
