use forge_foundation::ZoneType;

use super::token_effect_base::{TokenEffectBase, TOKEN_EFFECT_BASE};
use super::EffectContext;
use crate::ability::spell_ability_effect::get_target_players;
use crate::card::card_zone_table::CardZoneTable;

#[manabrew_engine_macros::spell_effect(RecruitEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    for p in get_target_players(ctx.game, sa) {
        super::draw_effect::draw_for_player(ctx, sa, p, 1);

        let hand: Vec<_> = ctx.game.cards_in_zone(ZoneType::Hand, p).to_vec();
        if !hand.is_empty()
            && !crate::staticability::static_ability_cant_discard::cant_discard(
                ctx.game,
                p,
                Some(sa),
                true,
            )
        {
            let amt = hand.len().min(1);
            let to_be_discarded = ctx.agents[p.index()].choose_discard(p, &hand, amt);
            let discarded_non_land = to_be_discarded
                .iter()
                .any(|&card_id| !ctx.game.card(card_id).is_land());
            for card_id in to_be_discarded {
                if ctx.game.card(card_id).zone == ZoneType::Hand {
                    ctx.discard_card(card_id, p, Some(sa));
                }
            }

            if discarded_non_land {
                let mut trigger_list = CardZoneTable::default();
                let result = TOKEN_EFFECT_BASE.make_token_table_from_scripts(
                    ctx,
                    &[p],
                    &["w_1_1_human_soldier".to_string()],
                    1,
                    false,
                    &mut trigger_list,
                    sa,
                );
                if !result.created.is_empty() {
                    trigger_list.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
                }
            }
        }
    }
}
