use forge_foundation::ZoneType;

use super::{matches_valid_cards_for_sa, EffectContext};

/// `SP$ ChooseCard` — player chooses card(s) from a filtered set in a zone.
///
/// Mirrors Java's `ChooseCardEffect.java`.
///
/// # Params
/// - `Amount` — how many cards to choose (default 1)
/// - `ChoiceZone` — zone to choose from (default Battlefield)
/// - `Choices` — ValidCards filter for eligible cards
/// - `RememberChosen` — if "True", add chosen to source's remembered_cards
///
/// Stores the chosen card(s) on the source card's `chosen_cards`.
/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `ChooseCardEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(ChooseCardEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let source_id = match sa.source {
        Some(id) => id,
        None => return,
    };

    let tgt_players =
        crate::ability::spell_ability_effect::get_defined_players_or_targeted(ctx.game, sa);

    let amount: usize = sa
        .ir
        .amount
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let min_amount: usize = sa
        .ir
        .min_amount
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(amount);
    if amount == 0 {
        return;
    }

    let zone = sa.ir.choice_zone.unwrap_or(ZoneType::Battlefield);

    let filter = sa.ir.choices.clone().unwrap_or_else(|| "Card".to_string());
    let filter_selector = sa.ir.choices_selector.clone();

    let remember = sa.ir.remember_chosen;

    // Collect valid cards in zone matching filter
    let mut valid = Vec::new();
    for &pid in &ctx.game.player_order.clone() {
        let zone_cards = ctx.game.cards_in_zone(zone, pid).to_vec();
        for cid in zone_cards {
            if matches_valid_cards_for_sa(
                ctx.game,
                sa,
                ctx.game.card(cid),
                filter_selector.as_ref(),
                &filter,
            ) {
                valid.push(cid);
            }
        }
    }
    if let Some(defined) = sa.ir.defined_cards.as_deref() {
        valid = crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
            ctx.game, sa, defined,
        );
    }

    let mut chosen = Vec::new();
    if let Some(total_power) = sa.ir.with_total_power {
        for p in tgt_players {
            chosen.extend(choose_with_total_power(ctx, sa, p, total_power));
        }
    } else {
        for p in tgt_players {
            ctx.agents[p.index()].snapshot_state(ctx.game, ctx.mana_pools);
            chosen.extend(
                ctx.agents[p.index()].choose_cards_for_effect(p, &valid, min_amount, amount),
            );
        }
    }

    // Store on source card
    ctx.game
        .card_mut(source_id)
        .set_chosen_cards(chosen.clone());

    // Optionally remember
    if remember {
        for &cid in &chosen {
            ctx.game.card_mut(source_id).add_remembered_card(cid);
        }
    }

    // ImprintChosen$ — `ChooseCardEffect.java:299-301`.
    if sa.ir.imprint_chosen {
        for &cid in &chosen {
            ctx.game.card_mut(source_id).add_imprinted_card(cid);
        }
    }
}

fn choose_with_total_power(
    ctx: &mut EffectContext,
    sa: &crate::spellability::SpellAbility,
    player: crate::ids::PlayerId,
    total_power: i32,
) -> Vec<crate::ids::CardId> {
    let creatures_with_power_le = |game: &crate::game::GameState,
                                   limit: i32,
                                   chosen_pool: &[crate::ids::CardId]|
     -> Vec<crate::ids::CardId> {
        game.cards_in_zone(ZoneType::Battlefield, player)
            .iter()
            .copied()
            .filter(|&cid| {
                let card = game.card(cid);
                card.is_creature() && card.power() <= limit && !chosen_pool.contains(&cid)
            })
            .collect()
    };
    let mut negative_creats = creatures_with_power_le(ctx.game, -1, &[]);
    let mut negative_num: i32 = negative_creats
        .iter()
        .map(|&cid| ctx.game.card(cid).power())
        .sum();
    let mut creature = creatures_with_power_le(ctx.game, total_power - negative_num, &[]);
    let mut chosen_pool: Vec<crate::ids::CardId> = Vec::new();
    let mut chosen_p = 0;
    while !creature.is_empty() {
        let options: Vec<crate::agent::GameEntity> = creature
            .iter()
            .map(|&cid| crate::agent::GameEntity::Card(cid))
            .collect();
        ctx.agents[player.index()].snapshot_state(ctx.game, ctx.mana_pools);
        match ctx.agents[player.index()].choose_single_entity_for_effect(
            player,
            &options,
            chosen_p <= total_power,
        ) {
            Some(crate::agent::GameEntity::Card(c)) => {
                chosen_p += ctx.game.card(c).power();
                chosen_pool.push(c);
                negative_creats.retain(|&n| n != c);
                negative_num = negative_creats
                    .iter()
                    .map(|&cid| ctx.game.card(cid).power())
                    .sum();
                creature = creatures_with_power_le(
                    ctx.game,
                    total_power - chosen_p - negative_num,
                    &chosen_pool,
                );
            }
            _ => {
                if ctx.agents[player.index()].confirm_action(
                    player,
                    Some("OptionalChoose"),
                    "Cancel choosing?",
                    &[],
                    sa.source,
                    sa.api,
                ) {
                    break;
                }
            }
        }
    }
    chosen_pool
}
