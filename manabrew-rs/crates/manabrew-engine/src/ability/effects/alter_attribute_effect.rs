//! AlterAttribute — change a creature's attribute (Plotted, Suspected, etc.).
//! Ported from Java's AlterAttributeEffect: toggles various card attributes
//! like Plotted, Harnessed, Solved, Suspected, Saddled, Commander.

use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `AlterAttributeEffect` class extending `SpellAbilityEffect`.
/// Java `AlterAttributeEffect`, `Prepared`: a token copy of the card in its prepared-spell
/// state goes straight into its owner's exile, and an effect lets the activator cast it
/// and unprepares the card when that copy is cast.
fn prepare(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility, card_id: CardId) {
    use forge_foundation::{CardTypeLine, ColorSet, ManaCost};

    let activator = sa.activating_player;
    let host = ctx.game.card(card_id).clone();
    let mut prepared =
        crate::ability::effects::copy_permanent_effect::get_proto_type(sa, &host, activator);
    prepared.other_part = host.other_part.clone();
    prepared.transform();
    prepared.full_name = prepared.card_name.clone();
    let prepared_id = ctx.game.create_card(prepared);
    let owner = ctx.game.card(prepared_id).owner;
    ctx.game.card_mut(prepared_id).zone = ZoneType::Exile;
    ctx.game
        .add_card_to_zone(ZoneType::Exile, owner, prepared_id);
    ctx.game.assign_zone_timestamp(prepared_id);

    let mut effect = crate::card::Card::new(
        CardId(0),
        format!("{}'s Prepared Spell", host.card_name),
        activator,
        CardTypeLine::parse("Effect"),
        ManaCost::parse("0"),
        ColorSet::COLORLESS,
        None,
        None,
        vec![],
        vec![],
    );
    effect.set_controller(activator);
    effect.set_effect_source(Some(card_id));
    effect.set_s_var(
        "PreparedUnprepare",
        "DB$ AlterAttribute | Defined$ EffectSource | Attributes$ Prepared | Activate$ False",
    );
    let mut next_id = 1000u32;
    if let Some(mut trigger) = crate::trigger::trigger::parse_trigger(
        "Mode$ SpellCast | TriggerZones$ Command | Static$ True | ValidSA$ Spell.IsRemembered \
         | Execute$ PreparedUnprepare",
        &mut next_id,
    ) {
        trigger.set_active_zone(vec![ZoneType::Command]);
        trigger.set_intrinsic(true);
        effect.add_trigger(trigger);
    }
    if let Some(mut may_play) = crate::staticability::parse_static_ability(
        "S$ Mode$ Continuous | MayPlay$ True | EffectZone$ Command | AffectedDefined$ Remembered \
         | AffectedZone$ Exile",
    ) {
        may_play.ir.active_zones = vec![ZoneType::Command];
        may_play.ir.has_zone_keys = true;
        may_play.base.set_intrinsic(true);
        effect.set_static_abilities(vec![may_play]);
    }
    effect.base_trigger_count = effect.triggers.len();
    effect.add_remembered_card(prepared_id);
    let effect_id = ctx.game.create_card(effect);
    ctx.move_card(effect_id, ZoneType::Command, activator);
    ctx.game.set_prepared(card_id, Some(effect_id));
}

#[manabrew_engine_macros::spell_effect(AlterAttributeEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let activate = sa.ir.alter_attribute_activate;
    let attributes = &sa.ir.alter_attribute_attributes;

    let targets: Vec<CardId> = if sa.uses_targeting() {
        sa.target_chosen.all_target_cards()
    } else if let Some(source) = sa.source {
        match sa.defined() {
            Some(defined) if !matches!(sa.defined_ref(), Some(DefinedRef::SelfCard)) => {
                crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game, sa, defined,
                )
            }
            _ => vec![source],
        }
    } else {
        return;
    };

    if sa.ir.optional {
        let activator = sa.activating_player;
        ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
        if !ctx.agents[activator.index()].confirm_action(
            activator,
            None,
            &sa.description,
            &[],
            sa.source,
            sa.api,
        ) {
            return;
        }
    }

    for card_id in targets {
        if ctx.game.card(card_id).zone == ZoneType::None {
            continue;
        }

        for attr in attributes {
            match attr.as_str() {
                "Harnessed" => {
                    let val = if activate { "True" } else { "False" };
                    ctx.game.card_mut(card_id).set_s_var("Harnessed", val);
                }
                "Plotted" => {
                    let turn = ctx.game.turn.turn_number;
                    crate::card::set_plotted(ctx.game.card_mut(card_id), activate, turn);
                    if activate {
                        ctx.trigger_handler.run_trigger(
                            TriggerType::BecomesPlotted,
                            RunParams {
                                card: Some(card_id),
                                ..Default::default()
                            },
                            false,
                        );
                    }
                }
                "Solve" | "Solved" => {
                    let val = if activate { "True" } else { "False" };
                    ctx.game.card_mut(card_id).set_s_var("Solved", val);
                    if activate {
                        ctx.trigger_handler.run_trigger(
                            TriggerType::CaseSolved,
                            RunParams {
                                card: Some(card_id),
                                player: Some(sa.activating_player),
                                ..Default::default()
                            },
                            false,
                        );
                    }
                }
                "Suspect" | "Suspected" => {
                    if activate {
                        // Suspected creatures have menace and can't block
                        if !ctx.game.card(card_id).keywords.contains_string("Menace") {
                            ctx.game.card_mut(card_id).add_intrinsic_keyword("Menace");
                        }
                        ctx.game.card_mut(card_id).set_s_var("Suspected", "True");
                    } else {
                        ctx.game
                            .card_mut(card_id)
                            .remove_intrinsic_keyword("Menace");
                        ctx.game.card_mut(card_id).remove_s_var("Suspected");
                    }
                }
                "Saddle" | "Saddled" => {
                    let first_time = ctx.game.card(card_id).get_s_var("Saddled") != Some("True");
                    let val = if activate { "True" } else { "False" };
                    ctx.game.card_mut(card_id).set_s_var("Saddled", val);
                    if activate {
                        ctx.trigger_handler.run_trigger(
                            TriggerType::BecomesSaddled,
                            RunParams {
                                card: Some(card_id),
                                player: Some(sa.activating_player),
                                first_time: Some(first_time),
                                ..Default::default()
                            },
                            false,
                        );
                    }
                }
                "Prepared" => {
                    if activate {
                        let card = ctx.game.card(card_id);
                        if card.is_prepared() || !card.has_prepared_spell_state() {
                            continue;
                        }
                        prepare(ctx, sa, card_id);
                    } else {
                        ctx.game.set_prepared(card_id, None);
                    }
                }
                "Commander" => {
                    let val = if activate { "True" } else { "False" };
                    ctx.game.card_mut(card_id).set_s_var("IsCommander", val);
                }
                other => crate::census::unhandled("alter-attribute-ignored", other),
            }

            if sa.ir.remember_altered {
                if let Some(source) = sa.source {
                    ctx.game.card_mut(source).add_remembered_card(card_id);
                }
            }
        }
    }
}
