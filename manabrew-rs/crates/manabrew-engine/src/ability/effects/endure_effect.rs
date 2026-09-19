//! Endure — creature endures: either put +1/+1 counters on it, or create
//! a Spirit token with power/toughness equal to the endure amount.
//! Ported from Java's EndureEffect.

use forge_foundation::ZoneType;

use super::token_effect_base::{TokenEffectBase, TOKEN_EFFECT_BASE};
use super::EffectContext;
use crate::card::card_zone_table::CardZoneTable;
use crate::ids::CardId;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `EndureEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(EndureEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let amount = super::resolve_numeric_svar(ctx.game, sa, "Num", 1).max(0);
    if amount < 1 {
        return; // CR 701.63b
    }

    let targets: Vec<CardId> = crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa);
    let targets =
        ctx.game
            .order_cards_by_their_owners(targets, ZoneType::Battlefield, &mut Some(ctx.agents));

    let mut token_table = super::token_effect_base::TokenCreateTable::default();
    for card_id in targets {
        let controller = ctx.game.card(card_id).controller;
        let counter_type = super::parse_counter_type("P1P1");
        let add_counters = ctx.game.card(card_id).zone == ZoneType::Battlefield
            && crate::card::card_predicates::can_receive_counters(ctx.game, card_id, &counter_type)
            && {
                ctx.agents[controller.index()].snapshot_state(ctx.game, ctx.mana_pools);
                let message = format!(
                    "Put {amount} +1/+1 counter(s) on {} (or create a {amount}/{amount} Spirit)?",
                    ctx.game.card(card_id).card_name
                );
                ctx.agents[controller.index()].confirm_action(
                    controller,
                    None,
                    &message,
                    &[],
                    Some(card_id),
                    Some(crate::ability::api_type::ApiType::Endure),
                )
            };
        if add_counters {
            ctx.game
                .card_mut(card_id)
                .add_counter(&counter_type, amount);
        } else {
            let mut token =
                TOKEN_EFFECT_BASE.require_token_template(ctx.token_templates, "w_x_x_spirit");
            token.set_owner(controller);
            token.set_controller(controller);
            token.set_is_token(true);
            token.set_s_var("TokenScript", "w_x_x_spirit");
            token.set_s_var("TokenSpawningAbility", sa.ability_text.clone());
            token.set_base_power(Some(amount));
            token.set_base_toughness(Some(amount));
            token_table.put(controller, token, 1);
        }
    }

    if !token_table.is_empty() {
        let mut trigger_list = CardZoneTable::default();
        let result =
            TOKEN_EFFECT_BASE.make_token_table(ctx, token_table, false, &mut trigger_list, sa);
        if !result.created.is_empty() {
            trigger_list.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
        }
    }
}
