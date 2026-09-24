use forge_foundation::{ColorSet, ZoneType};

use super::trait_animate_effect::parse_animate_params;
use super::{matches_valid_cards_for_sa, EffectContext};
use crate::card::card_changed_type::CardChangedType;
use crate::card::card_trait_changes::CardTraitChanges;
use crate::card::perpetual::perpetual_interface::PerpetualInterface;
use crate::card::perpetual::{
    perpetual_abilities, perpetual_colors, perpetual_incorporate, perpetual_keywords,
    perpetual_mana_cost, perpetual_new_pt, perpetual_types,
};
use crate::card::AnimateState;
use crate::parsing::keys;
use crate::trigger::parse_trigger;
use forge_foundation::ManaCost;

/// `SP$ AnimateAll` — animate all matching permanents on the battlefield.
///
/// Mirrors Java's `AnimateAllEffect.java`.
///
/// # Params
/// - `ValidCards` — filter for which cards to animate (e.g. "Land.YouCtrl", "Creature")
/// - `Power` — set base power
/// - `Toughness` — set base toughness
/// - `Types` — comma-separated types to add (e.g. "Creature,Elemental")
/// - `Keywords` — `&`-separated keywords to grant (until EOT)
/// - `Colors` — comma-separated colors to set (e.g. "Blue")
/// - `OverwriteColors` — if "True", replace color instead of adding
/// - `RemoveCreatureTypes` — if "True", remove creature types before adding new types
/// - `RemoveAllAbilities` — if "True", clear all keywords/abilities
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `AnimateAllEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(AnimateAllEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let valid_filter = sa
        .ir
        .valid_cards_text
        .clone()
        .unwrap_or_else(|| "Card".to_string());
    let valid_selector = sa.ir.valid_cards_selector.clone();

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
    // AnimateAll uses " & " as keyword separator (unlike Animate which uses ",")
    // parse_animate_params splits on "," so we also split on "&" for AnimateAll-specific handling
    let keywords_str = sa.ir.animate_keywords_text.clone();
    let triggers_str = sa.ir.animate_triggers_text.clone();
    let colors_str = anim_params.colors.map(|c| c.join(","));
    let overwrite_colors = sa.param_is_true(keys::OVERWRITE_COLORS);
    let remove_creature_types = sa.ir.animate_remove_creature_types;
    let remove_all_abilities = sa.ir.animate_remove_all_abilities;
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

    // Resolve Triggers$ SVars from source into parsed Trigger objects.
    let mut parsed_triggers: Vec<crate::trigger::Trigger> = Vec::new();
    if let Some(ref trigs) = triggers_str {
        let source_id = sa.source.unwrap_or(crate::ids::CardId(0));
        let source_svars = ctx.game.card(source_id).svars.clone();
        let mut next_trig_id = 1000u32;
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

    let player_ids = if !sa.uses_targeting() && sa.defined().is_none() {
        ctx.game.player_order.clone()
    } else {
        crate::ability::spell_ability_effect::get_target_players(ctx.game, sa)
    };
    let mut targets = Vec::new();
    for &pid in &player_ids {
        let zone_cards = ctx.game.cards_in_zone(ZoneType::Battlefield, pid).to_vec();
        for cid in zone_cards {
            if matches_valid_cards_for_sa(
                ctx.game,
                sa,
                ctx.game.card(cid),
                valid_selector.as_ref(),
                &valid_filter,
            ) {
                targets.push(cid);
            }
        }
    }

    let mut type_timestamp: Option<u64> = None;
    for card_id in targets {
        if ctx.game.card(card_id).zone != ZoneType::Battlefield {
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

        let until_registered = crate::ability::spell_ability_effect::add_until_command(
            ctx.game,
            sa.ir.duration.as_ref(),
            sa.activating_player,
            sa.source,
            crate::phase::PhaseCommand::RestoreAnimate { card: card_id },
        );

        // Save original state (only if not already animated this turn)
        if ctx.game.card(card_id).animate_state.is_none() {
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
                    ends_at_end_of_turn: !until_registered
                        && !is_permanent_duration
                        && !is_perpetual,
                }));
        }

        let mut added_abilities = Vec::new();
        if let Some(ref names) = sa.ir.abilities {
            let source_id = sa.source.unwrap_or(crate::ids::CardId(0));
            let source_svars = ctx.game.card(source_id).svars.clone();
            for name in names.split(',').map(str::trim) {
                if let Some(text) = source_svars.get(name) {
                    added_abilities.push(crate::spellability::build_spell_ability_from_host_card(
                        ctx.game.card(card_id),
                        text,
                        sa.activating_player,
                    ));
                    super::animate_effect::copy_execute_chain_svars(
                        &source_svars,
                        ctx.game.card_mut(card_id),
                        name,
                    );
                }
            }
        }

        if remove_all_abilities || !added_abilities.is_empty() {
            let card = ctx.game.card_mut(card_id);
            if remove_all_abilities {
                card.clear_pump_keywords();
                card.clear_static_changed_card_keywords();
            }
            card.add_changed_card_traits(
                CardTraitChanges {
                    abilities: added_abilities,
                    remove_all: remove_all_abilities,
                    ..Default::default()
                },
                resolve_ts,
                0,
            );
            if let Some(state) = card.animate_state.as_mut() {
                state.trait_change_timestamps.push(resolve_ts);
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
                keys::ADD_ALL_CREATURE_TYPES,
            ),
            remove_super_types: sa.ir.animate_remove_super_types,
            remove_card_types: sa.ir.animate_remove_card_types,
            remove_sub_types: sa.ir.animate_remove_sub_types,
            remove_creature_types,
        };
        if !changed_type.is_empty() {
            let timestamp = *type_timestamp.get_or_insert_with(|| ctx.game.next_timestamp());
            ctx.game
                .card_mut(card_id)
                .add_changed_card_types(changed_type, timestamp);
        }

        if let (Some(types), Some(ts)) = (&types_str, effect_ts) {
            for t in types.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                perpetual_types::PerpetualTypes {
                    timestamp: ts,
                    add_types: vec![t.to_string()],
                }
                .apply_effect(ctx.game.card_mut(card_id));
            }
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
            if let Some(val) = parsed_power {
                ctx.game.card_mut(card_id).set_base_power(Some(val));
            }
            if let Some(val) = parsed_toughness {
                ctx.game.card_mut(card_id).set_base_toughness(Some(val));
            }
        }

        // Apply keywords (until EOT — stored in pump_keywords so they get cleared at cleanup)
        // AnimateAll uses " & " as keyword separator (unlike Animate which uses ",")
        if let Some(ref kws) = keywords_str {
            for kw in kws.split('&') {
                let kw = kw.trim();
                if !kw.is_empty() {
                    if let Some(ts) = effect_ts {
                        perpetual_keywords::PerpetualKeywords {
                            timestamp: ts,
                            add_keywords: vec![kw.to_string()],
                            remove_keywords: Vec::new(),
                            remove_all: false,
                        }
                        .apply_effect(ctx.game.card_mut(card_id));
                    } else {
                        ctx.game.card_mut(card_id).add_pump_keyword(kw);
                    }
                }
            }
        }

        // Apply color
        if let Some(ref colors) = colors_str {
            let mut new_color = ColorSet::COLORLESS;
            for c in colors.split(',') {
                let c = c.trim().to_lowercase();
                match c.as_str() {
                    "white" | "w" => new_color = new_color.union(ColorSet::WHITE),
                    "blue" | "u" => new_color = new_color.union(ColorSet::BLUE),
                    "black" | "b" => new_color = new_color.union(ColorSet::BLACK),
                    "red" | "r" => new_color = new_color.union(ColorSet::RED),
                    "green" | "g" => new_color = new_color.union(ColorSet::GREEN),
                    _ => {}
                }
            }
            if overwrite_colors {
                if let Some(ts) = effect_ts {
                    perpetual_colors::PerpetualColors {
                        timestamp: ts,
                        colors: new_color,
                        overwrite: true,
                    }
                    .apply_effect(ctx.game.card_mut(card_id));
                } else {
                    ctx.game.card_mut(card_id).set_color(new_color);
                }
            } else if let Some(ts) = effect_ts {
                perpetual_colors::PerpetualColors {
                    timestamp: ts,
                    colors: new_color,
                    overwrite: false,
                }
                .apply_effect(ctx.game.card_mut(card_id));
            } else {
                let union = ctx.game.card(card_id).color.union(new_color);
                ctx.game.card_mut(card_id).set_color(union);
            }
        }

        // Apply trigger trait changes from Triggers$
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
                        if let Some(exec_svar) = source_svars.get(&trig.execute) {
                            ctx.game
                                .card_mut(card_id)
                                .set_s_var_if_absent(trig.execute.clone(), exec_svar.clone());
                        }
                    }
                }
            } else {
                for trig in &parsed_triggers {
                    ctx.game.card_mut(card_id).add_pump_trigger(trig.clone());
                    if !trig.execute.is_empty() {
                        if let Some(exec_svar) = source_svars.get(&trig.execute) {
                            ctx.game
                                .card_mut(card_id)
                                .set_s_var_if_absent(trig.execute.clone(), exec_svar.clone());
                        }
                    }
                }
            }
            ctx.trigger_handler
                .register_active_trigger(ctx.game, card_id);
        }
    }
}
