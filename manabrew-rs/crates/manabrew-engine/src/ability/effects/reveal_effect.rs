use forge_foundation::ZoneType;

use super::{resolve_numeric_svar, EffectContext};
use crate::agent::GameLogEvent;
use crate::ids::CardId;
use crate::parsing::{keys, Params};

/// Mirrors Java's `RevealEffect.java`.
#[manabrew_engine_macros::spell_effect(RevealEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let params = Params::from_raw(&sa.ability_text);
    let any_number = params.has("AnyNumber");
    let optional = params.has("Optional");
    let mut cnt = if params.has(keys::NUM_CARDS) {
        resolve_numeric_svar(ctx.game, sa, keys::NUM_CARDS, 1).max(0) as usize
    } else {
        1
    };
    let valid_in = |ctx: &EffectContext, cards: &[CardId], valid: &str| -> Vec<CardId> {
        cards
            .iter()
            .copied()
            .filter(|&cid| {
                crate::ability::ability_utils::matches_valid_cards_for_sa(
                    ctx.game,
                    sa,
                    ctx.game.card(cid),
                    None,
                    valid,
                )
            })
            .collect()
    };

    for p in crate::ability::spell_ability_effect::get_target_players(ctx.game, sa) {
        if !ctx.game.player(p).is_alive() {
            continue;
        }
        let cards_in_hand = ctx.game.cards_in_zone(ZoneType::Hand, p).to_vec();
        if cards_in_hand.is_empty() {
            continue;
        }
        let revealed: Vec<CardId> = if params.has("Random") {
            let valid = match params.get("RevealValid") {
                Some(filter) => valid_in(ctx, &cards_in_hand, filter),
                None => cards_in_hand,
            };
            if valid.is_empty() {
                continue;
            }
            let reveal_num = valid.len().min(cnt);
            let mut picked: Vec<CardId> = Vec::new();
            for (i, &cid) in valid.iter().enumerate() {
                if i < reveal_num {
                    picked.push(cid);
                } else {
                    let j = ctx.rng.next_int(i as i32 + 1) as usize;
                    if j < reveal_num {
                        picked[j] = cid;
                    }
                }
            }
            picked
        } else if let Some(defined) = params.get("RevealDefined") {
            crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                ctx.game, sa, defined,
            )
        } else if let Some(filter) = params.get("RevealAllValid") {
            valid_in(ctx, &cards_in_hand, filter)
        } else {
            let valid = match params.get("RevealValid") {
                Some(filter) => valid_in(ctx, &cards_in_hand, filter),
                None => cards_in_hand,
            };
            if valid.is_empty() {
                continue;
            }
            cnt = cnt.min(valid.len());
            let mut min = cnt;
            if any_number {
                cnt = valid.len();
                min = 0;
            } else if optional {
                min = 0;
            }
            ctx.agents[p.index()].choose_cards_to_reveal(p, &valid, min, cnt)
        };

        for agent in ctx.agents.iter_mut() {
            for &id in &revealed {
                let name = ctx.game.card(id).card_name.clone();
                agent.notify(crate::agent::notification::GameNotification::Event(
                    GameLogEvent::rule(format!("Revealed: {name}")).with_card(id),
                ));
            }
        }
        let source_name = sa.source.map(|cid| ctx.game.card(cid).card_name.clone());
        for agent in ctx.agents.iter_mut() {
            agent.reveal_cards(
                ctx.game,
                p,
                &revealed,
                ZoneType::Hand,
                p,
                source_name.as_deref(),
            );
        }
        if params.has("RememberRevealed") {
            if let Some(host) = sa.source {
                ctx.game
                    .card_mut(host)
                    .add_remembered_cards(revealed.iter().copied());
            }
        }
    }
}
