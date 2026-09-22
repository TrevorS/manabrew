use forge_foundation::ZoneType;

use super::helpers::matches_valid_cards_for_sa;
use super::EffectContext;
use crate::agent::GameEntity;
use crate::ids::{CardId, PlayerId};
use crate::parsing::Params;
use crate::spellability::SpellAbility;

/// `SP$ TwoPiles` — a separator divides cards into two piles and a chooser picks one.
///
/// Mirrors Java's `TwoPilesEffect.java`.
#[manabrew_engine_macros::spell_effect(TwoPilesEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let Some(source) = sa.source else {
        return;
    };
    let params = Params::from_raw(&sa.ability_text);
    let is_left_right_pile = params.has("LeftRightPile");
    let zone = params.get("Zone").and_then(ZoneType::from_str_compat);
    let valid = params.get("ValidCards").unwrap_or("Card").to_string();
    let face_down = params.get("FaceDown").unwrap_or("False").to_string();
    let target_players = crate::ability::spell_ability_effect::get_target_players(ctx.game, sa);

    let separator = choose_defined_player(ctx, sa, params.get("Separator"))
        .unwrap_or(ctx.game.card(source).controller);
    let Some(chooser) = choose_defined_player(ctx, sa, params.get("Chooser"))
        .or_else(|| target_players.first().copied())
    else {
        return;
    };

    for player in target_players {
        if !ctx.game.player(player).is_alive() {
            continue;
        }
        let (pile1, pile2) = if let Some(defined) = params.get("DefinedPiles") {
            let (first, second) = defined.split_once(',').unwrap_or((defined, ""));
            (
                crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game, sa, first,
                ),
                crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game, sa, second,
                ),
            )
        } else {
            let pool0: Vec<CardId> = if let Some(defined) = params.get("DefinedCards") {
                crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game, sa, defined,
                )
            } else {
                zone.map(|zone| ctx.game.cards_in_zone(zone, player).to_vec())
                    .unwrap_or_default()
            };
            let pool: Vec<CardId> = pool0
                .into_iter()
                .filter(|&cid| {
                    matches_valid_cards_for_sa(ctx.game, sa, ctx.game.card(cid), None, &valid)
                })
                .collect();
            if pool.is_empty() {
                return;
            }
            ctx.agents[separator.index()].snapshot_state(ctx.game, ctx.mana_pools);
            let pile1 = ctx.agents[separator.index()].choose_cards_for_effect(
                separator,
                &pool,
                0,
                pool.len(),
            );
            let pile2: Vec<CardId> = pool.into_iter().filter(|c| !pile1.contains(c)).collect();
            (pile1, pile2)
        };

        let pile1_chosen = is_left_right_pile || {
            ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
            ctx.agents[chooser.index()].choose_cards_pile(chooser, &pile1, &pile2, &face_down)
        };
        let (chosen_pile, unchosen_pile) = if pile1_chosen {
            (pile1, pile2)
        } else {
            (pile2, pile1)
        };

        if params.has("RememberChosen") {
            ctx.game
                .card_mut(source)
                .add_remembered_cards(chosen_pile.iter().copied());
        }
        if params.has("ChosenPile") {
            resolve_pile(ctx, sa, source, "ChosenPile", &chosen_pile);
        }
        if params.has("UnchosenPile") {
            resolve_pile(ctx, sa, source, "UnchosenPile", &unchosen_pile);
        }
    }

    if !params.has("KeepRemembered") && !params.has("RememberChosen") {
        ctx.game.card_mut(source).clear_remembered();
    }
}

fn choose_defined_player(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    defined: Option<&str>,
) -> Option<PlayerId> {
    let choices = crate::ability::ability_utils::resolve_defined_players_with_sa(
        defined?,
        sa,
        sa.activating_player,
        ctx.game,
    );
    if choices.is_empty() {
        return None;
    }
    let entities: Vec<GameEntity> = choices.into_iter().map(GameEntity::Player).collect();
    let activator = sa.activating_player;
    ctx.agents[activator.index()].snapshot_state(ctx.game, ctx.mana_pools);
    match ctx.agents[activator.index()].choose_single_entity_for_effect(activator, &entities, false)
    {
        Some(GameEntity::Player(pid)) => Some(pid),
        _ => None,
    }
}

fn resolve_pile(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    source: CardId,
    key: &str,
    pile: &[CardId],
) {
    let card = ctx.game.card_mut(source);
    let temp_cards = std::mem::take(&mut card.remembered_cards);
    let temp_players = std::mem::take(&mut card.remembered_players);
    card.add_remembered_cards(pile.iter().copied());
    if let Some(sub) = sa.additional_ability(ctx.game, key) {
        super::effect_resolver::resolve_effect_chain(ctx, sub);
    }
    let card = ctx.game.card_mut(source);
    card.remembered_cards.retain(|c| !pile.contains(c));
    card.remembered_cards.extend(temp_cards);
    card.remembered_players.extend(temp_players);
}
