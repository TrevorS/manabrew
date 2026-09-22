//! ManifestBaseEffect — abstract base for manifest variants.
//!
//! Mirrors Java's `ManifestBaseEffect.java`.
//! Provides shared logic for `ManifestEffect`, `ManifestDreadEffect`,
//! and `CloakEffect` that puts cards onto the battlefield face-down
//! as 2/2 creatures.

use forge_foundation::ZoneType;

use crate::ids::{CardId, PlayerId};
use crate::spellability::SpellAbility;

use super::EffectContext;

/// Common manifest parameters parsed from a spell ability.
pub struct ManifestParams {
    /// Number of cards to manifest.
    pub amount: usize,
    /// Whether the manifested cards come from the library.
    pub from_library: bool,
}

/// Parse common manifest parameters from a spell ability.
pub fn parse_manifest_params(ctx: &EffectContext, sa: &SpellAbility) -> ManifestParams {
    let amount = super::resolve_numeric_svar(ctx.game, sa, "Amount", 1).max(0) as usize;
    let from_library = sa
        .ir
        .defined_text
        .as_deref()
        .is_none_or(|d| d == "TopOfLibrary");
    ManifestParams {
        amount,
        from_library,
    }
}

pub fn manifest_target_cards(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    player: PlayerId,
    amount: usize,
) -> Option<Vec<CardId>> {
    if sa.ir.choices.is_some() || sa.ir.choice_zone.is_some() {
        let zone = sa.ir.choice_zone.unwrap_or(ZoneType::Hand);
        let mut choices = ctx.game.cards_in_zone(zone, player).to_vec();
        if let Some(filter) = sa.ir.choices.as_deref() {
            choices.retain(|&cid| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    sa.ir.choices_selector.as_ref(),
                    filter,
                )
            });
        }
        if choices.is_empty() {
            return None;
        }
        ctx.agents[player.index()].snapshot_state(ctx.game, ctx.mana_pools);
        return Some(
            ctx.agents[player.index()].choose_cards_for_effect(player, &choices, amount, amount),
        );
    }
    if sa.defined().is_none_or(|d| d == "TopOfLibrary") {
        let lib = ctx.game.cards_in_zone(ZoneType::Library, player).to_vec();
        return Some(lib.into_iter().rev().take(amount).collect());
    }
    Some(crate::ability::spell_ability_effect::get_target_cards(
        ctx.game, sa,
    ))
}

/// Get the default message for manifest choice prompts.
pub fn default_manifest_message() -> &'static str {
    "Choose a card to manifest"
}

/// Get the default message for manifest dread choice prompts.
pub fn default_manifest_dread_message() -> &'static str {
    "Choose a card to manifest dread"
}

/// Get the default message for cloak choice prompts.
pub fn default_cloak_message() -> &'static str {
    "Choose a card to cloak"
}

/// Resolve the base manifest effect — put cards from the top of the library
/// onto the battlefield face-down as 2/2 creatures.
/// Mirrors Java's `ManifestBaseEffect.resolve(SpellAbility)`.
///
/// This is the shared resolution logic for Manifest, Manifest Dread, and Cloak.
/// Each variant may override behavior (Manifest Dread looks at more cards,
/// Cloak grants the face-down card special turn-face-up abilities).
pub fn resolve(ctx: &mut EffectContext, sa: &SpellAbility, is_cloak: bool) {
    let player = sa.activating_player;
    let manifest = parse_manifest_params(ctx, sa);

    for _ in 0..manifest.amount {
        let library = ctx
            .game
            .cards_in_zone(forge_foundation::ZoneType::Library, player);
        if library.is_empty() {
            break;
        }

        // Take the top card of the library
        let card_id = library[0];

        // Move to battlefield face-down
        ctx.game
            .move_card(card_id, forge_foundation::ZoneType::Battlefield, player);

        // Set face-down properties — 2/2 creature with no abilities
        let card = ctx.game.card_mut(card_id);
        card.set_face_down(true);
        card.manifested = true;
        if is_cloak {
            card.cloaked = true;
        }

        // Set base P/T to 2/2 for face-down creatures
        card.add_new_pt(2, 2);

        // Register triggers for the new permanent
        ctx.trigger_handler
            .register_active_trigger(ctx.game, card_id);

        // Fire zone change triggers
        super::zone_triggers::emit_zone_trigger(
            ctx.trigger_handler,
            card_id,
            forge_foundation::ZoneType::Library,
            forge_foundation::ZoneType::Battlefield,
        );
    }
}
