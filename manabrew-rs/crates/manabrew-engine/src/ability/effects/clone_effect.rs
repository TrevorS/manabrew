use forge_foundation::{CardTypeLine, ColorSet, ZoneType};

use super::{matches_valid_cards_for_sa, EffectContext};
use crate::parsing::split_param_list_value;
use crate::spellability::SpellAbility;

/// `SP$ Clone` — one card becomes a copy of another.
///
/// Mirrors Java's `CloneEffect.java`.
///
/// # Params
/// - `Choices` — filter for valid clone sources (if player picks)
/// - `ChoiceZone` — zone to pick from (default Battlefield)
/// - `Defined$` — resolve defined cards as the clone source
/// - `CloneTarget` — defined cards to be cloned onto (default: source card)
/// - `PumpKeywords` — extra keywords on the copy
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `CloneEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(CloneEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    if !crate::ability::spell_ability_effect::check_valid_duration(
        ctx.game,
        sa,
        sa.ir.duration.as_ref(),
    ) {
        return;
    }
    let source_id = match sa.source {
        Some(id) => id,
        None => return,
    };

    let controller = sa.activating_player;

    // Step 1: Determine the clone source (what to copy FROM)
    let clone_source = resolve_clone_source(ctx, sa, controller);
    let clone_source_id = match clone_source {
        Some(id) => id,
        None => return,
    };

    if sa.ir.optional {
        let card_name = ctx.game.card(clone_source_id).card_name.clone();
        ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[controller.index()].confirm_action(
            controller,
            None,
            &format!("Do you want to copy {card_name}?"),
            &[],
            sa.source,
            Some(crate::ability::api_type::ApiType::Clone),
        ) {
            return;
        }
    }

    let clone_targets: Vec<crate::ids::CardId> =
        if let Some(defined) = sa.ir.clone_target.as_deref() {
            let mut targets = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                ctx.game, sa, defined,
            );
            if targets.is_empty() && defined == "ParentTarget" {
                targets.extend(ctx.parent_target_card);
            }
            if targets.is_empty() {
                return;
            }
            targets
        } else {
            vec![source_id]
        };

    let src = ctx.game.card(clone_source_id).clone();
    for clone_target_id in clone_targets {
        if ctx.game.card(clone_target_id).phased_out {
            continue;
        }
        // Step 3: Copy characteristics from source → target
        let duration = crate::parsing::raw_get(&sa.ability_text, crate::parsing::keys::DURATION);
        let active_animation = capture_active_animation(ctx.game.card(clone_target_id));
        if ctx.game.card(clone_target_id).clone_state.is_none() {
            let mut state = ctx.game.card(clone_target_id).capture_clone_state();
            state.expires_at_cleanup = duration.is_some() || sa.ir.duration.is_some();
            if let Some(animate_state) = ctx.game.card(clone_target_id).animate_state.as_ref() {
                state.original_type_line = animate_state.original_type_line.clone();
                state.original_base_power = animate_state.original_base_power;
                state.original_base_toughness = animate_state.original_base_toughness;
                state.original_color = animate_state.original_color;
            }
            ctx.game
                .card_mut(clone_target_id)
                .set_clone_state(Some(state));
        }
        // A card copying itself keeps its replacement ids, or the Moved replacement that
        // started the copy would no longer count as run and would fire again.
        let replacement_ids: Vec<i32> = src
            .replacement_effects
            .iter()
            .map(|re| {
                if clone_target_id == clone_source_id {
                    crate::core::Identifiable::id(&re.base.card_trait_base)
                } else {
                    ctx.game.next_copied_replacement_id()
                }
            })
            .collect();
        let target = &mut ctx.game.cards[clone_target_id.index()];
        let host_svars = (clone_target_id == source_id).then(|| target.svars.clone());
        crate::card::card_copy_service::copy_copiable_characteristics(&src, target);
        // Forge builds the sub-abilities and `Execute$` abilities of the cloning ability
        // before the copy; this engine looks them up by name on the host when they resolve.
        for (name, value) in host_svars.into_iter().flatten() {
            target.svars.entry(name).or_insert(value);
        }
        target.add_clone_state();
        target.activated_abilities = src.activated_abilities.clone();
        target.static_abilities = src.static_abilities.clone();
        target.replacement_effects = src.replacement_effects.clone();
        for static_ability in &mut target.static_abilities {
            static_ability.base.set_host_card_id(clone_target_id);
        }
        for (replacement_effect, id) in target.replacement_effects.iter_mut().zip(replacement_ids) {
            replacement_effect.base.set_host_card_id(clone_target_id);
            replacement_effect.base.card_trait_base.set_id(id);
        }
        target.ensure_crew_activated_ability();
        target.base_ability_count = target.activated_abilities.len();
        target.base_trigger_count = target.triggers.len();
        target.set_perpetual(&src, false);
        target.reset_changed_card_traits_baseline_to_current();
        let copied_name =
            if crate::parsing::raw_has_key(&sa.ability_text, crate::parsing::keys::KEEP_NAME) {
                target
                    .clone_state
                    .as_ref()
                    .map(|state| state.original_card_name.clone())
            } else {
                crate::parsing::raw_get(&sa.ability_text, crate::parsing::keys::NEW_NAME)
                    .map(str::to_string)
            };
        if let Some(name) = copied_name {
            if src.has_prepared_spell_state() {
                if let Some(other) = target.other_part.as_mut() {
                    other.name = name.clone();
                }
            }
            target.card_name = name;
        }

        // Step 4: Apply clone-state modifications from the cloning ability.
        if let Some(add_types) = sa.ir.add_types.as_deref() {
            for ty in split_param_list_value(Some(add_types), " & ") {
                ctx.game.card_mut(clone_target_id).add_type(&ty);
            }
        }

        if let Some(set_color) = sa.ir.set_color.as_deref() {
            ctx.game
                .card_mut(clone_target_id)
                .set_color(ColorSet::from_names(set_color));
        }

        if let Some(power) = sa
            .ir
            .set_power
            .as_deref()
            .and_then(|value| value.parse().ok())
        {
            ctx.game
                .card_mut(clone_target_id)
                .set_base_power(Some(power));
        }
        if let Some(toughness) = sa
            .ir
            .set_toughness
            .as_deref()
            .and_then(|value| value.parse().ok())
        {
            ctx.game
                .card_mut(clone_target_id)
                .set_base_toughness(Some(toughness));
        }

        if let Some(add_kws) = sa.ir.add_keywords.as_deref() {
            let keywords = add_kws.strip_prefix("IfNew ").unwrap_or(add_kws);
            for kw in split_param_list_value(Some(keywords), " & ") {
                ctx.game
                    .card_mut(clone_target_id)
                    .add_intrinsic_keyword(&kw);
            }
        }

        if sa.is_activated
            && crate::parsing::raw_has_key(
                &sa.ability_text,
                crate::parsing::keys::GAIN_THIS_ABILITY,
            )
        {
            let target = ctx.game.card_mut(clone_target_id);
            let index = target.activated_abilities.len();
            if let Some(ability) =
                crate::ability::activated::parse_activated_ability(&sa.ability_text, index)
            {
                target.activated_abilities.push(ability);
                target.base_ability_count = target.activated_abilities.len();
            }
        }

        if let Some(animation) = active_animation {
            reapply_active_animation(ctx.game.card_mut(clone_target_id), &animation);
        }

        // Step 5: Apply PumpKeywords$ (extra temporary keywords on the copy)
        if let Some(pump_kws) = sa.ir.pump_keywords.as_deref() {
            for kw in split_param_list_value(Some(pump_kws), " & ") {
                ctx.game
                    .card_mut(clone_target_id)
                    .add_intrinsic_keyword(&kw);
            }
        }

        {
            let target = ctx.game.card_mut(clone_target_id);
            target.remembered_cards.clear();
            target.imprinted_cards.clear();
            if crate::parsing::raw_has_key(
                &sa.ability_text,
                crate::parsing::keys::REMEMBER_CLONE_ORIGIN,
            ) {
                target.add_remembered(clone_source_id);
            }
        }

        // Step 6: Re-register triggers for the cloned card
        ctx.trigger_handler
            .register_active_trigger(ctx.game, clone_target_id);
    }
}

/// End-of-turn revert for clone effects. Mirrors the `GameCommand.run()`
/// anonymous class in Java `CloneEffect` that calls `removeCloneState`,
/// clears imprinted/remembered cards, and restores the original state.
///
/// Removes the clone stamp from the card, reverting copied characteristics
/// and restoring original remembered/imprinted state.
pub fn run(game: &mut crate::game::GameState, card_id: crate::ids::CardId) {
    if game.card(card_id).zone != ZoneType::Battlefield {
        return;
    }
    // Revert copiable characteristics by clearing clone-specific state.
    // The card's base characteristics (from the card definition) take over.
    let card = game.card_mut(card_id);
    card.remove_clone_state();
    card.imprinted_cards.clear();
    card.remembered_cards.clear();
}

/// Determine which card to copy FROM.
fn resolve_clone_source(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    controller: crate::ids::PlayerId,
) -> Option<crate::ids::CardId> {
    // Java `CloneEffect` asks Choices$ first, then Defined$, then the target.
    if let Some(filter) = sa.ir.choices.as_deref().map(str::to_string) {
        let filter_selector = sa.ir.choices_selector.as_ref();
        let zone = sa.ir.choice_zone.unwrap_or(ZoneType::Battlefield);

        let mut valid = Vec::new();
        for &pid in &ctx.game.player_order.clone() {
            let zone_cards = ctx.game.cards_in_zone(zone, pid).to_vec();
            for cid in zone_cards {
                if matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    filter_selector,
                    &filter,
                ) {
                    valid.push(cid);
                }
            }
        }

        ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let choices: Vec<crate::agent::GameEntity> = valid
            .iter()
            .copied()
            .map(crate::agent::GameEntity::Card)
            .collect();
        let choice_optional =
            crate::parsing::raw_has_key(&sa.ability_text, crate::parsing::keys::CHOICE_OPTIONAL);
        return match ctx.agents[controller.index()].choose_single_entity_for_effect(
            controller,
            &choices,
            choice_optional,
        ) {
            Some(crate::agent::GameEntity::Card(card_id)) => Some(card_id),
            _ => None,
        };
    }

    if let Some(defined) = sa.defined() {
        let mut sources = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
            ctx.game, sa, defined,
        );
        if sources.is_empty() {
            if let Some(parent) = ctx.parent_target_card {
                if defined == "ParentTarget" {
                    sources.push(parent);
                }
            }
        }
        return sources.first().copied();
    }

    if sa.uses_targeting() {
        return sa.target_chosen.target_card;
    }

    None
}

#[derive(Clone)]
struct ActiveAnimationSnapshot {
    original_type_line: CardTypeLine,
    original_color: ColorSet,
    original_base_power: Option<i32>,
    original_base_toughness: Option<i32>,
    original_keywords: Option<Vec<String>>,
    type_line: CardTypeLine,
    color: ColorSet,
    base_power: Option<i32>,
    base_toughness: Option<i32>,
    keywords: Vec<String>,
}

fn capture_active_animation(card: &crate::card::Card) -> Option<ActiveAnimationSnapshot> {
    let state = card.animate_state.as_ref()?;
    Some(ActiveAnimationSnapshot {
        original_type_line: state.original_type_line.clone(),
        original_color: state.original_color,
        original_base_power: state.original_base_power,
        original_base_toughness: state.original_base_toughness,
        original_keywords: state
            .original_keywords
            .as_ref()
            .map(|kws| kws.iter_strings().map(str::to_string).collect()),
        type_line: card.type_line.clone(),
        color: card.color,
        base_power: card.base_power,
        base_toughness: card.base_toughness,
        keywords: card.keywords.iter_strings().map(str::to_string).collect(),
    })
}

fn reapply_active_animation(card: &mut crate::card::Card, animation: &ActiveAnimationSnapshot) {
    for supertype in animation
        .type_line
        .supertypes
        .difference(&animation.original_type_line.supertypes)
    {
        card.type_line.supertypes.insert(*supertype);
    }
    for core_type in animation
        .type_line
        .core_types
        .difference(&animation.original_type_line.core_types)
    {
        card.type_line.core_types.insert(*core_type);
    }
    for subtype in &animation.type_line.subtypes {
        if animation
            .original_type_line
            .subtypes
            .iter()
            .any(|original| original.eq_ignore_ascii_case(subtype))
        {
            continue;
        }
        if !card
            .type_line
            .subtypes
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(subtype))
        {
            card.type_line.subtypes.push(subtype.clone());
        }
    }
    card.update_types();
    card.update_types_for_view();

    if animation.color != animation.original_color {
        card.color = animation.color;
    }
    if animation.base_power != animation.original_base_power {
        card.base_power = animation.base_power;
    }
    if animation.base_toughness != animation.original_base_toughness {
        card.base_toughness = animation.base_toughness;
    }

    let original_keywords = animation.original_keywords.as_deref().unwrap_or(&[]);
    for keyword in &animation.keywords {
        if original_keywords
            .iter()
            .any(|original| original.eq_ignore_ascii_case(keyword))
        {
            continue;
        }
        if !card
            .keywords
            .iter_strings()
            .any(|existing| existing.eq_ignore_ascii_case(keyword))
        {
            card.add_intrinsic_keyword(keyword);
        }
    }
}
