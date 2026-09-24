use forge_foundation::ZoneType;

use super::EffectContext;
use crate::ids::CardId;

#[manabrew_engine_macros::spell_effect(MustBlockEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(host) = sa.source else {
        return;
    };
    let cards: Vec<CardId> = match crate::parsing::raw_get(&sa.ability_text, "DefinedAttacker") {
        Some(defined) => {
            let cards = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                ctx.game, sa, defined,
            );
            if cards.is_empty() {
                return;
            }
            cards
        }
        None => vec![host],
    };

    let must_block_all = crate::parsing::raw_has_key(&sa.ability_text, "BlockAllDefined");

    for tgt in crate::ability::spell_ability_effect::get_target_cards(ctx.game, sa) {
        let card = ctx.game.card(tgt);
        if card.zone != ZoneType::Battlefield || card.phased_out {
            continue;
        }
        if must_block_all {
            ctx.game.card_mut(tgt).add_must_block_cards(cards.clone());
        } else {
            ctx.game.card_mut(tgt).add_must_block_card(cards[0]);
        }
    }
}
