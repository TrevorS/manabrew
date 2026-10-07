//! Ported from Java's `EmpowerEffect.java`.

use forge_foundation::ZoneType;

use super::token_effect_base::{TokenEffectBase, TOKEN_EFFECT_BASE};
use super::{parse_counter_type, EffectContext};
use crate::agent::{DecisionContext, GameEntity};
use crate::card::card_zone_table::CardZoneTable;
use crate::game::GameState;
use crate::ids::{CardId, PlayerId};

#[manabrew_engine_macros::spell_effect(EmpowerEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(&p) = crate::ability::spell_ability_effect::get_target_players(ctx.game, sa).first()
    else {
        return;
    };
    let amount = super::resolve_numeric_svar(ctx.game, sa, "Num", 1);
    let Some(empower_type) = sa.ir.type_filter.clone() else {
        return;
    };

    if typed_tokens(ctx.game, p, &empower_type).is_empty() {
        let mut trigger_list = CardZoneTable::default();
        let typed_script = format!("u_empower_{}", empower_type.to_lowercase());
        let script = if TOKEN_EFFECT_BASE
            .get_token_template(ctx.token_templates, &typed_script)
            .is_some()
        {
            typed_script
        } else {
            "u_empower".to_string()
        };
        let mut result = TOKEN_EFFECT_BASE.get_proto_type(ctx, &script, sa, p);
        result.add_type(&empower_type);
        result.card_name = format!("{empower_type} Token");

        let token_table = TOKEN_EFFECT_BASE.make_token_table_internal(p, result, 1);
        TOKEN_EFFECT_BASE.make_token_table(ctx, token_table, false, &mut trigger_list, sa);
        trigger_list.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
    }

    let tgt_cards = typed_tokens(ctx.game, p, &empower_type);
    if tgt_cards.is_empty() {
        return;
    }

    ctx.agents[p.index()].snapshot_state(ctx.game, ctx.mana_pools);
    let entities: Vec<GameEntity> = tgt_cards.iter().copied().map(GameEntity::Card).collect();
    let tgt = match ctx.agents[p.index()].choose_single_entity_for_effect(
        DecisionContext::new(ctx.game, ctx.mana_pools),
        p,
        &entities,
        false,
    ) {
        Some(GameEntity::Card(card)) => card,
        _ => tgt_cards[0],
    };

    ctx.add_counter(
        tgt,
        &parse_counter_type("LOYALTY"),
        amount,
        sa,
        crate::event::RunParams {
            source_player: Some(p),
            ..Default::default()
        },
    );
}

fn typed_tokens(game: &GameState, p: PlayerId, empower_type: &str) -> Vec<CardId> {
    game.cards
        .iter()
        .filter(|c| {
            c.zone == ZoneType::Battlefield
                && c.controller == p
                && c.is_token
                && c.has_string_type(empower_type)
        })
        .map(|c| c.id)
        .collect()
}
