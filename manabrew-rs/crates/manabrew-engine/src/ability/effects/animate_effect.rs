use forge_foundation::{ColorSet, ZoneType};

use super::trait_animate_effect::parse_animate_params;
use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;
use crate::card::card_changed_type::CardChangedType;
use crate::card::card_trait_changes::CardTraitChanges;
use crate::card::perpetual::perpetual_interface::PerpetualInterface;
use crate::card::perpetual::{
    perpetual_abilities, perpetual_colors, perpetual_incorporate, perpetual_keywords,
    perpetual_mana_cost, perpetual_new_pt, perpetual_types,
};
use crate::card::AnimateState;
use crate::spellability::SpellAbility;
use crate::trigger::parse_trigger;
use forge_foundation::ManaCost;

/// `SP$ Animate` — turn a non-creature permanent into a creature (or modify creature stats).
///
/// Mirrors Java's `AnimateEffectBase.java` + `AnimateEffect.java`.
///
/// # Params
/// - `Defined$` — target card(s) (default: source card itself)
/// - `Power` — set base power
/// - `Toughness` — set base toughness
/// - `Types` — comma-separated types to add (e.g. "Creature,Land")
/// - `Keywords` — comma-separated keywords to grant (until EOT)
/// - `Colors` — comma-separated colors to set (e.g. "White,Blue")
/// - `OverwriteTypes` — if "True", replace type_line instead of adding
///
/// The animate_state is saved so step_cleanup can restore the original card state.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `AnimateEffect` class extending `SpellAbilityEffect`.
/// Java builds an `Execute$` ability with its whole `SubAbility$` chain attached, while
/// this port looks each one up by name on the host when the trigger resolves, so a
/// granted trigger needs every SVar in the chain, not just the first.
pub(super) fn copy_execute_chain_svars(
    source_svars: &std::collections::BTreeMap<String, String>,
    card: &mut crate::card::Card,
    start: &str,
) {
    let mut name = start.to_string();
    for _ in 0..16 {
        let Some(text) = source_svars.get(&name) else {
            return;
        };
        let text = text.clone();
        card.set_s_var_if_absent(name.clone(), text.clone());
        for (_, value) in crate::parsing::Params::from_raw(&text).iter() {
            let value = value.trim();
            if let Some(referenced) = source_svars.get(value) {
                card.set_s_var_if_absent(value.to_string(), referenced.clone());
            }
        }
        let Some(next) = crate::parsing::raw_get(&text, crate::parsing::keys::SUB_ABILITY) else {
            return;
        };
        name = next.trim().to_string();
    }
}

#[manabrew_engine_macros::spell_effect(AnimateEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    if !crate::ability::spell_ability_effect::check_valid_duration(
        ctx.game,
        sa,
        sa.ir.duration.as_ref(),
    ) {
        return;
    }
    let controller = sa.activating_player;

    // Determine target card
    let target_ids = resolve_animate_targets(ctx, sa, controller);

    if sa.ir.optional {
        ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[controller.index()].confirm_action(
            controller,
            None,
            "Do you want to animate?",
            &[],
            sa.source,
            Some(crate::ability::api_type::ApiType::Animate),
        ) {
            return;
        }
    }

    // Use shared base-class parsing for common animate params
    let anim_params = parse_animate_params(sa);
    let power_str = sa
        .ir
        .animate_power_text
        .as_deref()
        .map(|raw| super::resolve_numeric_value(ctx.game, sa, raw, 0).to_string());
    let toughness_str = sa
        .ir
        .animate_toughness_text
        .as_deref()
        .map(|raw| super::resolve_numeric_value(ctx.game, sa, raw, 0).to_string());
    let types_str = if anim_params.add_types.is_empty() {
        None
    } else {
        Some(anim_params.add_types.join(","))
    };
    let keywords_str = if anim_params.add_keywords.is_empty() {
        None
    } else {
        Some(anim_params.add_keywords.join(","))
    };
    let remove_keywords_str = sa.ir.animate_remove_keywords_text.clone();
    let colors_str = anim_params.colors.map(|c| c.join(","));
    let triggers_str = sa.ir.animate_triggers_text.clone();
    let overwrite_types = anim_params.overwrite_types;
    let incorporate_cost = sa.ir.animate_incorporate_text.clone();
    let mana_cost_override = sa.ir.animate_mana_cost_override_text.clone();
    let is_perpetual = matches!(
        sa.ir.duration,
        Some(crate::spellability::AbilityDuration::Perpetual)
    );
    let is_permanent_duration = matches!(
        sa.ir.duration,
        Some(crate::spellability::AbilityDuration::Permanent)
    );
    let resolve_ts = ctx.game.next_effect_timestamp();

    // Resolve Triggers$ SVars from the source card into parsed Trigger objects.
    // These will be temporarily added to each target card.
    let mut parsed_triggers: Vec<crate::trigger::Trigger> = Vec::new();
    if let Some(ref trigs) = triggers_str {
        // The source card holds the SVars that define the triggers.
        let source_id = sa.source.unwrap_or(crate::ids::CardId(0));
        let source_svars = ctx.game.card(source_id).svars.clone();
        let mut next_trig_id = 1000u32; // high base to avoid collisions
        for trig_name in trigs.split(',') {
            let trig_name = trig_name.trim();
            if trig_name.is_empty() {
                continue;
            }
            if let Some(svar_text) = source_svars.get(trig_name) {
                if let Some(mut trig) = parse_trigger(svar_text, &mut next_trig_id) {
                    trig.execute = trig.execute.clone();
                    trig.original_host = sa.original_host.or(sa.source);
                    parsed_triggers.push(trig);
                }
            }
        }
    }

    let mut parsed_statics: Vec<crate::staticability::StaticAbility> = Vec::new();
    if let Some(ref names) = sa.ir.animate_static_abilities_text {
        let source_id = sa.source.unwrap_or(crate::ids::CardId(0));
        let source_svars = &ctx.game.card(source_id).svars;
        for name in names.split(',') {
            if let Some(text) = source_svars.get(name.trim()) {
                if let Some(st) = crate::staticability::parse_static_ability(text) {
                    parsed_statics.push(st);
                }
            }
        }
    }

    // Snapshot target IDs for AtEOT$ delayed trigger registration after the loop.
    let eot_targets = target_ids.clone();

    let mut type_timestamp: Option<u64> = None;
    for card_id in target_ids {
        let card = ctx.game.card(card_id);
        if card.phased_out
            || (sa.source == Some(card_id)
                && !matches!(card.zone, ZoneType::Battlefield | ZoneType::Stack))
        {
            continue;
        }

        let effect_ts = if is_perpetual { Some(resolve_ts) } else { None };

        if let Some(ref mc) = incorporate_cost {
            let ts = resolve_ts;
            perpetual_incorporate::PerpetualIncorporate {
                timestamp: ts,
                incorporate: ManaCost::parse(mc),
            }
            .apply_effect(ctx.game.card_mut(card_id));
        }
        if let Some(ref mc) = mana_cost_override {
            if let Some(ts) = effect_ts {
                perpetual_mana_cost::PerpetualManaCost {
                    timestamp: ts,
                    mana_cost: ManaCost::parse(mc),
                }
                .apply_effect(ctx.game.card_mut(card_id));
            }
        }

        if is_permanent_duration {
            ctx.game
                .card_mut(card_id)
                .capture_changed_characteristics_baseline_if_needed();
        }

        let until_registered = crate::ability::spell_ability_effect::add_until_command(
            ctx.game,
            sa.ir.duration.as_ref(),
            sa.activating_player,
            sa.source,
            crate::phase::PhaseCommand::RestoreAnimate { card: card_id },
        );

        // Save original state (only if not already animated this turn)
        if !is_permanent_duration && !is_perpetual && ctx.game.card(card_id).animate_state.is_none()
        {
            let original_type_line = ctx
                .game
                .card(card_id)
                .static_type_line_base
                .clone()
                .unwrap_or_else(|| ctx.game.card(card_id).type_line.clone());
            let original_base_power = ctx.game.card(card_id).base_power;
            let original_base_toughness = ctx.game.card(card_id).base_toughness;
            let original_color = ctx.game.card(card_id).color;
            let original_keywords = ctx.game.card(card_id).keywords.clone();
            ctx.game
                .card_mut(card_id)
                .set_animate_state(Some(AnimateState {
                    original_type_line,
                    original_base_power,
                    original_base_toughness,
                    original_color,
                    original_keywords: Some(original_keywords),
                    trait_change_timestamps: Vec::new(),
                    ends_at_end_of_turn: !until_registered,
                    new_power: None,
                    new_toughness: None,
                    new_color: None,
                }));
        }

        let removes_all_abilities = sa.ir.animate_remove_all_abilities && effect_ts.is_none();

        if overwrite_types && types_str.is_some() {
            let card = ctx.game.card_mut(card_id);
            card.set_type_line(forge_foundation::CardTypeLine::new());
            if is_permanent_duration {
                if let Some(state) = card.animate_state.as_mut() {
                    state.original_type_line = forge_foundation::CardTypeLine::new();
                }
            }
        }

        let changed_type = CardChangedType {
            add_type: match (&types_str, effect_ts) {
                (Some(types), None) => types
                    .split(',')
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            },
            add_all_creature_types: crate::parsing::raw_has_key(
                &sa.ability_text,
                crate::parsing::keys::ADD_ALL_CREATURE_TYPES,
            ),
            remove_super_types: sa.ir.animate_remove_super_types,
            remove_card_types: sa.ir.animate_remove_card_types,
            remove_sub_types: sa.ir.animate_remove_sub_types,
            remove_creature_types: sa.ir.animate_remove_creature_types,
        };
        if !changed_type.is_empty() {
            let timestamp = *type_timestamp.get_or_insert_with(|| ctx.game.next_timestamp());
            let card = ctx.game.card_mut(card_id);
            if is_permanent_duration {
                if let Some(state) = card.animate_state.as_mut() {
                    changed_type.apply_changes(&mut state.original_type_line);
                }
            }
            card.add_changed_card_types(changed_type, timestamp);
        }

        if let (Some(types), Some(ts)) = (&types_str, effect_ts) {
            for t in types.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                perpetual_types::PerpetualTypes {
                    timestamp: ts,
                    add_types: vec![t.to_string()],
                }
                .apply_effect(ctx.game.card_mut(card_id));
            }
            let card = ctx.game.card_mut(card_id);
            if crate::staticability::layer::sanitize_subtypes(&mut card.type_line) {
                card.update_types();
            }
        }

        if let Some(name) = sa.ir.name_text.as_deref() {
            let card = ctx.game.card_mut(card_id);
            if card.changed_name_base.is_none() {
                card.changed_name_base = Some(card.card_name.clone());
            }
            card.set_card_name(name);
        }

        if sa
            .ir
            .spell_description_text
            .as_deref()
            .is_some_and(|desc| desc.to_ascii_lowercase().starts_with("crew"))
        {
            let crew: Vec<crate::ids::CardId> = sa
                .paid_hash
                .get(crate::cost::cost_tap_type::HASH_CARDS)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| {
                            value
                                .strip_prefix("Card#")
                                .unwrap_or(value)
                                .parse::<u32>()
                                .ok()
                                .map(crate::ids::CardId)
                        })
                        .collect()
                })
                .unwrap_or_default();
            ctx.game.card_mut(card_id).becomes_crewed(&crew);
            let first_time = ctx.game.card(card_id).times_crewed_this_turn == 1;
            ctx.trigger_handler.run_trigger(
                crate::trigger::TriggerType::BecomesCrewed,
                crate::event::RunParams {
                    card: Some(card_id),
                    crew_cards: Some(crew),
                    first_time: Some(first_time),
                    ..Default::default()
                },
                false,
            );
        }

        // Apply P/T
        let parsed_power = power_str
            .as_deref()
            .and_then(|p| p.trim().parse::<i32>().ok());
        let parsed_toughness = toughness_str
            .as_deref()
            .and_then(|t| t.trim().parse::<i32>().ok());
        if let Some(ts) = effect_ts {
            if parsed_power.is_some() || parsed_toughness.is_some() {
                perpetual_new_pt::PerpetualNewPt {
                    timestamp: ts,
                    power: parsed_power,
                    toughness: parsed_toughness,
                }
                .apply_effect(ctx.game.card_mut(card_id));
            }
        } else {
            let card = ctx.game.card_mut(card_id);
            if let Some(val) = parsed_power {
                card.set_base_power(Some(val));
            }
            if let Some(val) = parsed_toughness {
                card.set_base_toughness(Some(val));
            }
            if let Some(state) = card.animate_state.as_mut() {
                if !is_permanent_duration {
                    state.add_new_pt(parsed_power, parsed_toughness);
                } else {
                    if parsed_power.is_some() {
                        state.original_base_power = parsed_power;
                    }
                    if parsed_toughness.is_some() {
                        state.original_base_toughness = parsed_toughness;
                    }
                }
            }
        }

        // Apply keyword changes. Permanent-duration animate effects need to mutate
        // the card's live keyword set (e.g. Animate Dead changing its Enchant text),
        // not just temporary pump keywords.
        let add_keywords: Vec<String> = keywords_str
            .as_deref()
            .map(|kws| {
                kws.split(',')
                    .map(str::trim)
                    .filter(|kw| !kw.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let remove_keywords: Vec<String> = remove_keywords_str
            .as_deref()
            .map(|kws| {
                kws.split(',')
                    .map(str::trim)
                    .filter(|kw| !kw.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let mut trait_keywords = Vec::new();
        if removes_all_abilities {
            trait_keywords = add_keywords;
        } else if !add_keywords.is_empty() || !remove_keywords.is_empty() {
            if let Some(ts) = effect_ts {
                perpetual_keywords::PerpetualKeywords {
                    timestamp: ts,
                    add_keywords,
                    remove_keywords,
                    remove_all: false,
                }
                .apply_effect(ctx.game.card_mut(card_id));
            } else if is_permanent_duration {
                let card = ctx.game.card_mut(card_id);
                for kw in &remove_keywords {
                    card.remove_changed_card_keywords(kw);
                    card.remove_hidden_extrinsic_keywords(kw);
                    card.remove_pump_keyword(kw);
                }
                for kw in &add_keywords {
                    card.add_changed_card_keywords(kw);
                }
            } else {
                let timestamp = ctx.game.next_timestamp();
                let card = ctx.game.card_mut(card_id);
                for kw in &remove_keywords {
                    card.remove_pump_keyword(kw);
                }
                if until_registered {
                    trait_keywords = add_keywords;
                } else {
                    for kw in &add_keywords {
                        card.add_pump_keyword(kw, timestamp);
                    }
                }
            }
        }

        // Apply triggers (e.g. Supernatural Stamina's death → return trigger)
        // Triggers are added from the SOURCE card's SVars, not the target's.
        // Also copy the SVars needed by the trigger's Execute$ reference.
        if !parsed_triggers.is_empty() {
            let source_id = sa.source.unwrap_or(crate::ids::CardId(0));
            let source_svars = ctx.game.card(source_id).svars.clone();
            if let Some(ts) = effect_ts {
                let changes = CardTraitChanges {
                    triggers: parsed_triggers.clone(),
                    ..Default::default()
                };
                perpetual_abilities::PerpetualAbilities {
                    timestamp: ts,
                    changes,
                }
                .apply_effect(ctx.game.card_mut(card_id));
                for trig in &parsed_triggers {
                    if !trig.execute.is_empty() {
                        copy_execute_chain_svars(
                            &source_svars,
                            ctx.game.card_mut(card_id),
                            &trig.execute,
                        );
                    }
                }
            } else {
                for trig in &parsed_triggers {
                    if is_permanent_duration {
                        ctx.game.card_mut(card_id).add_lasting_trigger(trig.clone());
                    } else {
                        ctx.game.card_mut(card_id).add_pump_trigger(trig.clone());
                    }
                    // Copy the Execute SVar from source to target so trigger resolution
                    // can find it (e.g. SupernaturalStaminaTrigChangeZone)
                    if !trig.execute.is_empty() {
                        copy_execute_chain_svars(
                            &source_svars,
                            ctx.game.card_mut(card_id),
                            &trig.execute,
                        );
                    }
                }
            }
            // Re-register this card's triggers so the new ones are active
            ctx.trigger_handler
                .register_active_trigger(ctx.game, card_id);
        }

        let mut added_abilities = Vec::new();
        if removes_all_abilities {
            let card = ctx.game.card_mut(card_id);
            card.clear_pump_keywords();
            card.clear_static_changed_card_keywords();
            let first_keyword_ability = card.activated_abilities.len();
            card.generate_keyword_activated_abilities(&trait_keywords);
            let card_controller = card.controller;
            for ab in card.activated_abilities.split_off(first_keyword_ability) {
                let mut ability =
                    crate::ability::ability_factory::build_spell_ability_from_host_card(
                        card,
                        &ab.ability_text,
                        card_controller,
                    );
                ability.is_activated = true;
                added_abilities.push(ability);
            }
        }
        if let Some(names) = sa.ir.abilities.as_deref() {
            let source_svars = ctx
                .game
                .card(sa.source.unwrap_or(crate::ids::CardId(0)))
                .svars
                .clone();
            for name in names.split(',').map(str::trim) {
                if let Some(text) = source_svars.get(name) {
                    added_abilities.push(crate::spellability::build_spell_ability_from_host_card(
                        ctx.game.card(card_id),
                        text,
                        controller,
                    ));
                    copy_execute_chain_svars(&source_svars, ctx.game.card_mut(card_id), name);
                }
            }
        }

        if removes_all_abilities
            || !added_abilities.is_empty()
            || !parsed_statics.is_empty()
            || !trait_keywords.is_empty()
        {
            let changes = CardTraitChanges {
                abilities: added_abilities,
                static_abilities: parsed_statics.clone(),
                keywords: trait_keywords,
                remove_all: removes_all_abilities,
                ..Default::default()
            };
            if let Some(ts) = effect_ts {
                perpetual_abilities::PerpetualAbilities {
                    timestamp: ts,
                    changes,
                }
                .apply_effect(ctx.game.card_mut(card_id));
            } else {
                let card = ctx.game.card_mut(card_id);
                card.add_changed_card_traits(changes, resolve_ts, 0);
                if !is_permanent_duration {
                    if let Some(state) = card.animate_state.as_mut() {
                        state.trait_change_timestamps.push(resolve_ts);
                    }
                }
            }
        }

        // Apply color
        if let Some(ref colors) = colors_str {
            let new_color = if colors == "ChosenColor" {
                ColorSet::from_mask(sa.source.map_or(0, |src| {
                    ctx.game
                        .card(src)
                        .chosen_colors
                        .iter()
                        .filter_map(|name| forge_foundation::Color::from_name(name))
                        .fold(0u8, |acc, color| acc | color.mask())
                }))
            } else if colors == "All" {
                ColorSet::ALL_COLORS
            } else {
                ColorSet::from_mask(
                    colors
                        .split(',')
                        .filter_map(|name| forge_foundation::Color::from_name(name.trim()))
                        .fold(0u8, |acc, color| acc | color.mask()),
                )
            };
            if let Some(ts) = effect_ts {
                perpetual_colors::PerpetualColors {
                    timestamp: ts,
                    colors: new_color,
                    overwrite: true,
                }
                .apply_effect(ctx.game.card_mut(card_id));
            } else {
                let card = ctx.game.card_mut(card_id);
                card.set_color(new_color);
                if let Some(state) = card.animate_state.as_mut() {
                    if is_permanent_duration {
                        state.original_color = new_color;
                    } else {
                        state.add_color(new_color, false);
                    }
                }
            }
        }
    }

    // `AtEOT$ <action>` — register an end-of-turn delayed trigger on the
    // animated cards still on the battlefield.
    if let Some(action) = sa.ir.at_eot.as_deref() {
        let remembered: Vec<crate::ids::CardId> = eot_targets
            .iter()
            .copied()
            .filter(|&cid| ctx.game.card(cid).zone == ZoneType::Battlefield)
            .collect();
        if !remembered.is_empty() {
            crate::ability::spell_ability_effect::register_at_eot(
                ctx.trigger_handler,
                ctx.game,
                sa,
                action,
                remembered,
            );
        }
    }
}

/// Resolve which card(s) to animate.
fn resolve_animate_targets(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    _controller: crate::ids::PlayerId,
) -> Vec<crate::ids::CardId> {
    let mut targets = crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa);
    if targets.is_empty() && matches!(sa.defined_ref(), Some(DefinedRef::ParentTarget)) {
        targets.extend(ctx.parent_target_card);
    }
    if let Some(stack_id) = sa
        .target_chosen
        .target_stack_entry
        .filter(|_| sa.uses_targeting())
    {
        if let Some(source) = ctx
            .game
            .stack
            .find_by_id(stack_id)
            .and_then(|entry| entry.spell_ability.source)
        {
            targets.push(source);
        }
    }
    targets
}
