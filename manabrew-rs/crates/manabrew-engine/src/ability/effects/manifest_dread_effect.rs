//! ManifestDread effect — look at top 2 cards, manifest one, other goes to graveyard.
//!
//! Ported from Java's `ManifestDreadEffect.java`.
//! Manifest Dread N: For each, look at top 2, choose one to manifest, rest to graveyard.

use forge_foundation::ZoneType;

use super::manifest_base_effect::parse_manifest_params;
use super::manifest_effect::manifest_single_card;
use super::{emit_zone_trigger, EffectContext};
use crate::agent::GameEntity;
use crate::event::RunParams;
use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ManifestDreadEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ManifestDreadEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let manifest_params = parse_manifest_params(ctx, sa);
    let amount = manifest_params.amount;
    let controller = sa.activating_player;

    let players = if let Some(def) = sa.defined_player() {
        super::resolve_defined_players(def, controller, ctx.game)
    } else {
        vec![controller]
    };

    for pid in players {
        for _ in 0..amount {
            manifest_dread_once(ctx, sa, pid);
        }
    }
}

/// One iteration of manifest dread: look at top 2, pick one to manifest, rest to graveyard.
fn manifest_dread_once(ctx: &mut EffectContext, sa: &SpellAbility, player: PlayerId) {
    let lib = ctx.game.cards_in_zone(ZoneType::Library, player).to_vec();
    // Top 2 cards (last 2 in the vec since top = end)
    let mut tgt_cards: Vec<CardId> = lib.into_iter().rev().take(2).collect();
    let mut to_grave = Vec::new();
    if !tgt_cards.is_empty() {
        ctx.agents[player.index()].snapshot_state(ctx.game, ctx.mana_pools);
        let entities: Vec<GameEntity> = tgt_cards.iter().copied().map(GameEntity::Card).collect();
        let manifest = match ctx.agents[player.index()]
            .choose_single_entity_for_effect(player, &entities, false)
        {
            Some(GameEntity::Card(cid)) => cid,
            _ => tgt_cards[0],
        };
        tgt_cards.retain(|&cid| cid != manifest);

        manifest_single_card(ctx, sa, manifest, player);
        let card = ctx.game.card(manifest);
        if !(card.manifested && card.zone == ZoneType::Battlefield) {
            tgt_cards.push(manifest);
        }
        for card_id in tgt_cards {
            let gz = ctx.game.card(card_id).zone;
            ctx.move_card(card_id, ZoneType::Graveyard, player);
            emit_zone_trigger(ctx.trigger_handler, card_id, gz, ZoneType::Graveyard);
            to_grave.push(card_id);
        }
    }
    ctx.trigger_handler.run_trigger(
        TriggerType::ManifestDread,
        RunParams {
            player: Some(player),
            cards: Some(to_grave),
            ..Default::default()
        },
        true,
    );
}
