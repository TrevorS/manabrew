use super::EffectContext;
use crate::agent::BinaryChoiceKind;
use crate::event::RunParams;
use crate::ids::{CardId, PlayerId};
use crate::parsing::Params;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

#[manabrew_engine_macros::spell_effect(FlipCoinEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(host) = sa.source else {
        return;
    };
    let params = Params::from_raw(&sa.ability_text);
    let players_to_flip = crate::ability::ability_utils::resolve_defined_players_with_sa(
        params.get("Flipper").unwrap_or("You"),
        sa,
        sa.activating_player,
        ctx.game,
    );
    let no_call = sa.ir.no_call;
    let for_each_player = params.get("ForEachPlayer").map(str::to_string);
    let var_name = params.get("SaveNumFlipsToSVar").unwrap_or("X").to_string();
    let has_amount = params.has("Amount");
    let amount = if has_amount {
        super::resolve_numeric_svar(ctx.game, sa, "Amount", 1)
    } else {
        1
    };

    for flipper in players_to_flip {
        if no_call {
            let count_heads = flip_coins(ctx, flipper, sa, amount);
            let count_tails = (count_heads - amount).abs();
            if count_heads > 0 {
                if params.has("RememberResult") {
                    ctx.game.card_mut(host).add_flip_result(true);
                }
                resolve_counted_sub(
                    ctx,
                    sa,
                    host,
                    "HeadsSubAbility",
                    has_amount.then_some((var_name.as_str(), count_heads)),
                );
            }
            if count_tails > 0 {
                if params.has("RememberResult") {
                    ctx.game.card_mut(host).add_flip_result(false);
                }
                resolve_counted_sub(
                    ctx,
                    sa,
                    host,
                    "TailsSubAbility",
                    has_amount.then_some((var_name.as_str(), count_tails)),
                );
            }
        } else if let Some(for_each) = for_each_player.as_deref() {
            let mut won_for = Vec::new();
            let mut lost_for = Vec::new();
            for p in crate::ability::ability_utils::resolve_defined_players_with_sa(
                for_each,
                sa,
                sa.activating_player,
                ctx.game,
            ) {
                if flip_coins(ctx, flipper, sa, 1) > 0 {
                    won_for.push(p);
                } else {
                    lost_for.push(p);
                }
            }
            if !won_for.is_empty() {
                resolve_with_remembered_players(ctx, sa, host, "WinSubAbility", &won_for, "Wins");
            }
            if !lost_for.is_empty() {
                resolve_with_remembered_players(
                    ctx,
                    sa,
                    host,
                    "LoseSubAbility",
                    &lost_for,
                    "Losses",
                );
            }
        } else {
            let count_wins = flip_coins(ctx, flipper, sa, amount);
            let count_losses = (count_wins - amount).abs();
            if count_wins > 0 {
                if params.has("RememberWinner") {
                    ctx.game.card_mut(host).add_remembered_player(flipper);
                }
                resolve_counted_sub(ctx, sa, host, "WinSubAbility", Some(("Wins", count_wins)));
            }
            if count_losses > 0 {
                if params.has("RememberLoser") {
                    ctx.game.card_mut(host).add_remembered_player(flipper);
                }
                resolve_counted_sub(
                    ctx,
                    sa,
                    host,
                    "LoseSubAbility",
                    Some(("Losses", count_losses)),
                );
            }
            if let Some(to_remember) = params.get("RememberNumber") {
                if to_remember.starts_with("Win") {
                    ctx.game.card_mut(host).remembered_cmc.push(count_wins);
                } else if to_remember.starts_with("Loss") {
                    ctx.game.card_mut(host).remembered_cmc.push(count_losses);
                }
            }
        }
    }
}

fn resolve_counted_sub(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    host: CardId,
    key: &str,
    count_svar: Option<(&str, i32)>,
) {
    if let Some(sub) = sa.get_additional_ability(key).cloned() {
        if let Some((name, count)) = count_svar {
            ctx.game
                .card_mut(host)
                .svars
                .insert(name.to_string(), format!("Number${count}"));
        }
        resolve_sub_chain(ctx, sub);
    }
}

fn resolve_with_remembered_players(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    host: CardId,
    key: &str,
    players: &[PlayerId],
    count_svar: &str,
) {
    let Some(sub) = sa.get_additional_ability(key).cloned() else {
        return;
    };
    let card = ctx.game.card_mut(host);
    let temp_cards = std::mem::take(&mut card.remembered_cards);
    let temp_players = std::mem::take(&mut card.remembered_players);
    let temp_numbers = std::mem::take(&mut card.remembered_cmc);
    card.add_remembered_players(players.iter().copied());
    card.svars
        .insert(count_svar.to_string(), format!("Number${}", players.len()));
    resolve_sub_chain(ctx, sub);
    let card = ctx.game.card_mut(host);
    card.remembered_players.retain(|p| !players.contains(p));
    card.remembered_cards.extend(temp_cards);
    card.remembered_players.extend(temp_players);
    card.remembered_cmc.extend(temp_numbers);
}

pub fn flip_coins(
    ctx: &mut EffectContext,
    flipper: PlayerId,
    sa: &SpellAbility,
    amount: i32,
) -> i32 {
    let multiplier = get_flip_multiplier(ctx, flipper);
    let mut result = 0;
    loop {
        let mut won = false;
        for _ in 0..amount {
            won = flip_coin(ctx, flipper, sa, multiplier);
            if won {
                result += 1;
            }
        }
        if !(sa.ir.flip_until_you_lose && won) {
            break;
        }
    }
    result
}

fn flip_coin(
    ctx: &mut EffectContext,
    flipper: PlayerId,
    sa: &SpellAbility,
    multiplier: i32,
) -> bool {
    let no_call = sa.ir.no_call;
    let mut choice = true;
    if !no_call {
        choice = ctx.agents[flipper.index()].choose_binary(
            flipper,
            "Call the coin flip",
            BinaryChoiceKind::HeadsOrTails,
            None,
            sa.source,
            sa.api,
        );
    }
    let mut saw_true = false;
    let mut saw_false = false;
    for _ in 0..multiplier {
        if ctx.rng.next_boolean() {
            saw_true = true;
        } else {
            saw_false = true;
        }
    }
    let result = if saw_true && saw_false {
        choice
    } else {
        saw_true
    };
    let won_or_heads = result == choice;
    ctx.game.player_mut(flipper).num_flips_this_turn += 1;
    if !no_call {
        ctx.trigger_handler.run_trigger(
            TriggerType::FlippedCoin,
            RunParams {
                player: Some(flipper),
                coin_flip_won: Some(won_or_heads),
                ..Default::default()
            },
            false,
        );
    }
    won_or_heads
}

/// Get the flip multiplier for a player. Each instance of "If you would flip
/// a coin, instead flip two coins and ignore one." doubles the flips.
/// Mirrors Java `FlipCoinEffect.getFlipMultiplier`.
/// Mirrors Java `FlipCoinEffect.getFlipMultiplier(Player)`.
pub fn get_flip_multiplier(ctx: &EffectContext, flipper: crate::ids::PlayerId) -> i32 {
    let keyword = "If you would flip a coin, instead flip two coins and ignore one.";
    let count = ctx
        .game
        .cards
        .iter()
        .filter(|c| {
            c.zone == forge_foundation::ZoneType::Battlefield
                && c.controller == flipper
                && c.keywords.contains_string_ignore_case(keyword)
        })
        .count() as u32;
    1i32 << count
}

fn resolve_sub_chain(ctx: &mut EffectContext, initial: SpellAbility) {
    let mut cur_opt: Option<SpellAbility> = Some(initial);
    while let Some(cur_sa) = cur_opt {
        super::resolve_effect(ctx, &cur_sa);
        cur_opt = cur_sa.sub_ability.map(|b| *b);
        if ctx.game.game_over {
            break;
        }
    }
}
