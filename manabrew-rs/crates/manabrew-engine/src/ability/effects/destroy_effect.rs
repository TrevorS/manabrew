use forge_foundation::ZoneType;

use super::EffectContext;
use crate::card::card_util;
use crate::event::RunParams;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `DestroyEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(DestroyEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let no_regen = sa.ir.no_regen;
    let remember_destroyed = sa.ir.remember_destroyed;
    let always_remember = sa.ir.always_remember;
    let remember_lki = sa.ir.remember_lki_flag;

    if remember_destroyed {
        if let Some(sid) = sa.source {
            ctx.game.card_mut(sid).clear_remembered();
        }
    }

    let untargeted: Vec<_> = card_util::get_radiance(ctx.game, sa)
        .iter()
        .copied()
        .collect();
    let tgt_cards = crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa);
    let tgt_cards = ctx.game.order_cards_by_their_owners(
        tgt_cards,
        ZoneType::Graveyard,
        &mut Some(&mut *ctx.agents),
    );
    let untargeted = ctx.game.order_cards_by_their_owners(
        untargeted,
        ZoneType::Graveyard,
        &mut Some(&mut *ctx.agents),
    );

    for target_card in tgt_cards.into_iter().chain(untargeted) {
        if ctx.game.card(target_card).zone == ZoneType::Battlefield {
            let is_indestructible = ctx.game.card(target_card).has_indestructible();
            let has_regen_shield = ctx.game.card(target_card).regeneration_shields > 0;

            // Indestructible prevents destruction (CR 702.12)
            if is_indestructible {
                // `AlwaysRemember` remembers the target even when destruction is
                // prevented (Java `AbilityUtils.setCauseSA` path for "that card"
                // references in chained subs).
                if always_remember && remember_lki {
                    if let Some(sid) = sa.source {
                        ctx.game.card_mut(sid).add_remembered_card(target_card);
                    }
                }
                continue;
            }

            // Regeneration (CR 701.15): consume a shield instead of destroying,
            // unless `NoRegen$ True` suppresses the replacement.
            if has_regen_shield && !no_regen {
                ctx.game.card_mut(target_card).regeneration_shields -= 1;
                // Regenerating taps the creature and removes it from combat.
                ctx.game.card_mut(target_card).tapped = true;
                if always_remember && remember_lki {
                    if let Some(sid) = sa.source {
                        ctx.game.card_mut(sid).add_remembered_card(target_card);
                    }
                }
                continue;
            }
            // Capture +1/+1 counter count before move (for Modular death triggers)
            let lki_p1p1 = *ctx
                .game
                .card(target_card)
                .counters
                .get(&crate::card::CounterType::P1P1)
                .unwrap_or(&0);

            // Capture LKI counters for death triggers (e.g. Servant of the Scale)
            {
                let lki_counters = ctx.game.card(target_card).counters.clone();
                let lki_power = ctx.game.card(target_card).power();
                let lki_toughness = ctx.game.card(target_card).toughness();
                ctx.game.card_mut(target_card).lki_counters = Some(lki_counters);
                ctx.game
                    .card_mut(target_card)
                    .set_lki_power_toughness(Some(lki_power), Some(lki_toughness));
            }
            // Fire Destroyed trigger before moving to graveyard
            ctx.trigger_handler.run_trigger(
                TriggerType::Destroyed,
                RunParams {
                    card: Some(target_card),
                    causer: sa.source,
                    cause_card: sa.source,
                    cause_player: Some(sa.activating_player),
                    ..Default::default()
                },
                false,
            );

            let lki_power = ctx
                .game
                .card(target_card)
                .lki_power
                .unwrap_or_else(|| ctx.game.card(target_card).power());
            let lki_toughness = ctx
                .game
                .card(target_card)
                .lki_toughness
                .unwrap_or_else(|| ctx.game.card(target_card).toughness());
            ctx.sacrifice_destroy(target_card, lki_p1p1, lki_power, lki_toughness);

            // Track the destroyed card on the source so chained sub-abilities
            // (`Destroyed` triggers in `EffectEffect`, "that card" references)
            // can find it.
            if remember_destroyed || remember_lki {
                if let Some(sid) = sa.source {
                    ctx.game.card_mut(sid).add_remembered_card(target_card);
                }
            }
        }
    }
}
