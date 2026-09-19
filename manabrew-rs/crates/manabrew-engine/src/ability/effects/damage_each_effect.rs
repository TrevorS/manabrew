use forge_foundation::ZoneType;

use super::{matches_valid_cards_for_sa, EffectContext};
use crate::card::card_damage_map::{CardDamageMap, DamageTarget};
use crate::ids::CardId;
use crate::parsing::Params;

/// Mirrors Java's `DamageEachEffect.java`.
#[manabrew_engine_macros::spell_effect(DamageEachEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(host) = sa.source else {
        return;
    };
    let params = Params::from_raw(&sa.ability_text);
    let num = params.get("NumDmg").unwrap_or("X").to_string();

    let sources: Vec<CardId> = if let Some(defined) = params.get("DefinedDamagers") {
        crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(ctx.game, sa, defined)
    } else {
        let battlefield: Vec<CardId> = ctx
            .game
            .player_order
            .clone()
            .iter()
            .flat_map(|&pid| ctx.game.cards_in_zone(ZoneType::Battlefield, pid).to_vec())
            .collect();
        match params.get("ValidCards") {
            Some(valid) => battlefield
                .into_iter()
                .filter(|&cid| {
                    matches_valid_cards_for_sa(
                        ctx.game,
                        sa,
                        ctx.game.card(cid),
                        sa.ir.valid_cards_selector.as_ref(),
                        valid,
                    )
                })
                .collect(),
            None => battlefield,
        }
    };

    let amount = |ctx: &EffectContext, source: CardId| -> i32 {
        let expr = ctx
            .game
            .card(host)
            .get_s_var(&num)
            .unwrap_or(&num)
            .to_string();
        let mut source_sa = sa.clone();
        source_sa.source = Some(source);
        crate::svar::resolve_numeric_value(ctx.game, &source_sa, &expr, 0)
    };

    let mut entries: Vec<(CardId, DamageTarget, i32)> = Vec::new();
    if params.has("EachToItself") {
        for &source in &sources {
            entries.push((source, DamageTarget::Card(source), amount(ctx, source)));
        }
    } else if let Some(defined) = params.get("ToEachOther") {
        let targets = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
            ctx.game, sa, defined,
        );
        for &damager in &targets {
            for &c in &targets {
                if c != damager {
                    entries.push((damager, DamageTarget::Card(c), amount(ctx, damager)));
                }
            }
        }
    } else {
        let mut targets: Vec<DamageTarget> = Vec::new();
        if sa.uses_targeting() {
            targets.extend(
                sa.target_chosen
                    .all_target_cards()
                    .into_iter()
                    .map(DamageTarget::Card),
            );
            targets.extend(
                sa.target_chosen
                    .all_target_players()
                    .into_iter()
                    .map(DamageTarget::Player),
            );
        } else {
            for defined in params.get("Defined").unwrap_or("Self").split(" & ") {
                let (players, cards) =
                    crate::ability::ability_utils::get_defined_entities(defined, sa, ctx.game);
                targets.extend(players.into_iter().map(DamageTarget::Player));
                targets.extend(cards.into_iter().map(DamageTarget::Card));
            }
        }
        for target in targets {
            if let DamageTarget::Card(c) = target {
                let card = ctx.game.card(c);
                if card.zone != ZoneType::Battlefield || card.phased_out {
                    continue;
                }
            }
            for &source in &sources {
                entries.push((source, target, amount(ctx, source)));
            }
        }
    }

    if ctx.game.pending_damage_map.is_some() {
        if let Some(map) = ctx.game.pending_damage_map.as_mut() {
            for (source, target, dmg) in entries {
                map.put(source, target, dmg);
            }
        }
        return;
    }
    let mut map = CardDamageMap::default();
    for (source, target, dmg) in entries {
        map.put(source, target, dmg);
    }
    let mut deal_sa = sa.clone();
    deal_sa.damage_map = Some(map);
    deal_sa.prevent_map = Some(CardDamageMap::default());
    super::damage_resolve_effect::DamageResolveEffect::resolve(ctx, &deal_sa);
}
