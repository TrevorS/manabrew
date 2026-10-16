//! Airbend — exile target cards, their owner may cast them for {2}.
//! Mirrors Java `AirbendEffect`: exiles each card and creates a per-card
//! Effect in the Command zone that grants `MayPlay` with an alternate
//! mana cost of `{2}`, so the owner may cast the exiled card from exile.

use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ability::ability_ir::DefinedRef;
use crate::ids::CardId;
use crate::staticability::parse_static_ability;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `AirbendEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(AirbendEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let parent_target = ctx.parent_target_card.or(sa.target_chosen.target_card);
    let primary_def = sa.ir.defined.as_ref().and_then(|d| d.refs.first());
    let targets: Vec<CardId> = match primary_def {
        None => {
            if sa.uses_targeting() {
                sa.target_chosen.target_card.into_iter().collect()
            } else if let Some(source) = sa.source {
                vec![source]
            } else {
                return;
            }
        }
        Some(DefinedRef::SelfCard) => sa.source.into_iter().collect(),
        Some(DefinedRef::Remembered) => sa
            .source
            .map(|sid| ctx.game.host_object(sid, sa).remembered_cards.clone())
            .unwrap_or_default(),
        Some(DefinedRef::Targeted) => sa.target_chosen.target_card.into_iter().collect(),
        Some(DefinedRef::Unsupported(raw)) if raw.starts_with("Valid ") => {
            // `Defined$ Valid <selector>` (Avatar's Wrath uses
            // `Valid Creature.NotDefinedTargeted`). Java's
            // `AirbendEffect` resolves these via `getCardsfromTargets`,
            // which delegates to `knownDetermineDefined`.
            let filter = raw.strip_prefix("Valid ").unwrap_or("").trim();
            let stripped: String = filter
                .split('.')
                .map(|seg| {
                    seg.split('+')
                        .filter(|term| !term.eq_ignore_ascii_case("NotDefinedTargeted"))
                        .collect::<Vec<_>>()
                        .join("+")
                })
                .filter(|seg| !seg.is_empty())
                .collect::<Vec<_>>()
                .join(".");
            let mut out = Vec::new();
            for &pid in &ctx.game.player_order {
                for &cid in ctx.game.cards_in_zone(ZoneType::Battlefield, pid) {
                    if filter.contains("NotDefinedTargeted")
                        && parent_target.is_some_and(|t| t == cid)
                    {
                        continue;
                    }
                    if crate::ability::ability_utils::matches_valid_cards_for_sa(
                        ctx.game,
                        sa,
                        ctx.game.card(cid),
                        None,
                        &stripped,
                    ) {
                        out.push(cid);
                    }
                }
            }
            out
        }
        _ => sa.source.into_iter().collect(),
    };

    let mut moved_any = false;
    for card_id in targets {
        if ctx.game.card(card_id).zone == ZoneType::None {
            continue;
        }

        // Exile the card
        let old_zone = ctx.game.card(card_id).zone;
        ctx.exile(card_id, Some(sa));
        super::emit_zone_trigger(ctx.trigger_handler, card_id, old_zone, ZoneType::Exile);
        moved_any |= ctx.game.card(card_id).zone == ZoneType::Exile;

        if ctx.game.card(card_id).is_token {
            continue;
        }
        if ctx.game.card(card_id).zone != ZoneType::Exile {
            continue;
        }

        create_airbend_effect(ctx, sa, card_id);
    }

    if moved_any {
        crate::player::trigger_elemental_bend(
            ctx.game,
            ctx.trigger_handler,
            sa.activating_player,
            crate::trigger::TriggerType::Airbend,
        );
    }
}

/// Java `AirbendEffect.resolve`'s per-card effect: its owner may cast the exiled card for {2}.
pub(crate) fn create_airbend_effect(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    exiled_card_id: CardId,
) {
    let owner = ctx.game.card(exiled_card_id).owner;
    let card_name = ctx.game.card(exiled_card_id).card_name.clone();
    let effect_id = crate::ability::spell_ability_effect::create_effect(
        ctx.game,
        sa,
        owner,
        &format!("Airbend {card_name}"),
        "",
    );
    let static_text = "S$ Mode$ Continuous | MayPlay$ True | MayPlayAltManaCost$ 2 \
        | EffectZone$ Command | Affected$ Card.IsRemembered+nonLand \
        | AffectedZone$ Exile | Description$ You may cast the card.";
    let effect = ctx.game.card_mut(effect_id);
    if let Some(static_ab) = parse_static_ability(static_text) {
        effect.add_static_ability(static_ab);
    }
    effect.add_remembered_cards([exiled_card_id]);
    effect.set_forget_on_moved_origin(Some(ZoneType::Exile));
    ctx.move_card(effect_id, ZoneType::Command, owner);
}
