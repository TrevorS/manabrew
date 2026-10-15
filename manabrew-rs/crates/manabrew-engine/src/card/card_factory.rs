use forge_carddb::CardRules;
use forge_foundation::{ColorSet, ZoneType};

use super::card_assembly;
use super::card_clone_states::CardCloneStates;
use super::{Card, CardOtherPart};
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};
use crate::parsing::{keys, raw_get, raw_has_key, split_param_list_value};
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
pub fn get_clone_states(
    input: &Card,
    new_owner: PlayerId,
    cause: &SpellAbility,
) -> CardCloneStates {
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
        input.state_color,
        input.base_power,
        input.base_toughness,
        input.copiable_keywords().as_string_list(),
        input.abilities.clone(),
    );
    if input.face_down {
        out.activated_abilities
            .retain(|ability| !ability.is_turn_face_up());
        out.base_ability_count = out.activated_abilities.len();
    }
    out.oracle_text = input.oracle_text.clone();
    let (mut triggers, printed): (Vec<_>, Vec<_>) = input
        .copiable_triggers()
        .into_iter()
        .partition(|trigger| trigger.base.card_trait_base.get_keyword().is_some());
    triggers.extend(printed);
    for (id, trigger) in triggers.iter_mut().enumerate() {
        trigger.id = id as u32;
    }
    out.set_triggers(triggers);
    out.set_svars_map(input.svars.clone());
    let set_color =
        cause.has_param(crate::parsing::keys::SET_COLOR) || cause.has_param("SetColorByManaCost");
    let set_creature_types = cause.has_param(crate::parsing::keys::SET_CREATURE_TYPES);
    let mut static_abilities = input.copiable_static_abilities();
    static_abilities.retain(|sta| {
        !sta.ir.characteristic_defining
            || !(cause.has_param(crate::parsing::keys::SET_POWER) && sta.ir.set_power
                || cause.has_param(crate::parsing::keys::SET_TOUGHNESS) && sta.ir.set_toughness
                || set_color && sta.ir.set_color
                || set_creature_types && sta.ir.add_all_creature_types)
    });
    out.set_static_abilities(static_abilities);
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
    update_clone_state(&mut out, input, cause);
    if let Some(other) = out.other_part.as_mut() {
        update_other_clone_state(other, cause);
    }
    if set_creature_types {
        out.remove_intrinsic_keyword("Changeling");
    }
    if set_color {
        out.remove_intrinsic_keyword("Devoid");
    }
    CardCloneStates::new(input.id, cause, out)
}

fn clone_colors(cause: &SpellAbility) -> Option<ColorSet> {
    cause.ir.set_color.as_deref().map(|set_color| {
        set_color
            .split(',')
            .map(ColorSet::from_names)
            .fold(ColorSet::COLORLESS, ColorSet::union)
    })
}

fn clone_keywords(cause: &SpellAbility) -> (bool, Vec<String>) {
    let Some(add_keywords) = cause.ir.add_keywords.as_deref() else {
        return (false, Vec::new());
    };
    let (if_new, keywords) = match add_keywords.strip_prefix("IfNew ") {
        Some(keywords) => (true, keywords),
        None => (false, add_keywords),
    };
    (if_new, split_param_list_value(Some(keywords), " & "))
}

fn clone_power_toughness(cause: &SpellAbility) -> (Option<i32>, Option<i32>) {
    let parse = |value: Option<&str>| value.and_then(|value| value.parse().ok());
    (
        parse(cause.ir.set_power.as_deref()),
        parse(cause.ir.set_toughness.as_deref()),
    )
}

fn clone_removes_mana_cost(cause: &SpellAbility) -> bool {
    cause
        .ir
        .set_mana_cost
        .as_deref()
        .is_some_and(|value| value == "0" || value.is_empty())
}

fn update_clone_state(state: &mut Card, input: &Card, cause: &SpellAbility) {
    if let Some(name) = raw_get(&cause.ability_text, keys::NEW_NAME) {
        state.card_name = name.to_string();
    }
    if let Some(colors) = clone_colors(cause) {
        state.set_color(colors);
    }
    if raw_has_key(&cause.ability_text, "NonLegendary") {
        state.remove_type("Legendary");
    }
    if let Some(add_types) = cause.ir.add_types.as_deref() {
        for ty in split_param_list_value(Some(add_types), " & ") {
            state.add_type(&ty);
        }
        if crate::staticability::layer::sanitize_subtypes(&mut state.state_type_line) {
            state.update_types();
        }
    }
    if let Some(creature_types) = cause.ir.set_creature_types.as_deref() {
        state.set_creature_types(creature_types);
    }
    let (if_new, keywords) = clone_keywords(cause);
    for kw in keywords {
        let (keyword, _) = crate::keyword::keyword_collection::parse_keyword_string(&kw);
        if !(if_new && state.keywords.contains_keyword(keyword)) {
            state.add_intrinsic_keyword_with_triggers(&kw);
        }
    }
    let (power, toughness) = clone_power_toughness(cause);
    if state.state_type_line.is_creature() || input.base_power.is_some() {
        if power.is_some() {
            state.set_base_power(power);
        }
        if toughness.is_some() {
            state.set_base_toughness(toughness);
        }
    }
    if clone_removes_mana_cost(cause) {
        state.set_mana_cost(forge_foundation::mana::ManaCost::no_cost());
    }
}

fn update_other_clone_state(other: &mut CardOtherPart, cause: &SpellAbility) {
    if let Some(name) = raw_get(&cause.ability_text, keys::NEW_NAME) {
        other.name = name.to_string();
    }
    if let Some(colors) = clone_colors(cause) {
        other.color = colors;
    }
    if raw_has_key(&cause.ability_text, "NonLegendary") {
        other
            .type_line
            .supertypes
            .retain(|st| !st.name().eq_ignore_ascii_case("Legendary"));
    }
    if let Some(add_types) = cause.ir.add_types.as_deref() {
        for ty in split_param_list_value(Some(add_types), " & ") {
            other.type_line.add_type(&ty);
        }
        crate::staticability::layer::sanitize_subtypes(&mut other.type_line);
    }
    if let Some(creature_types) = cause.ir.set_creature_types.as_deref() {
        super::card_state::set_creature_types_of(&mut other.type_line, creature_types);
    }
    let (if_new, keywords) = clone_keywords(cause);
    for kw in keywords {
        let (keyword, _) = crate::keyword::keyword_collection::parse_keyword_string(&kw);
        if !(if_new && other.keywords.contains_keyword(keyword)) {
            other.keywords.add(&kw);
        }
    }
    let (power, toughness) = clone_power_toughness(cause);
    if other.type_line.is_creature() || other.base_power.is_some() {
        if power.is_some() {
            other.base_power = power;
        }
        if toughness.is_some() {
            other.base_toughness = toughness;
        }
    }
    if clone_removes_mana_cost(cause) {
        other.mana_cost = forge_foundation::mana::ManaCost::no_cost();
    }
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
            game.host_object_mut(source, source_sa)
                .add_remembered_card(copy_id);
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
