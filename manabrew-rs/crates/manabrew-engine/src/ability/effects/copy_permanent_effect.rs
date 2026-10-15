use forge_foundation::ZoneType;

use super::token_effect_base::{TokenCreateTable, TokenEffectBase, TOKEN_EFFECT_BASE};
use super::EffectContext;
use crate::agent::DecisionContext;
use crate::card::card_zone_table::CardZoneTable;
use crate::card::Card;
use crate::ids::CardId;
use crate::spellability::SpellAbility;

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `CopyPermanentEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(CopyPermanentEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let amount = super::resolve_numeric_svar(ctx.game, sa, "NumCopies", 1).max(0) as usize;
    let controllers = resolve_copy_controllers(ctx, sa);
    if controllers.is_empty() {
        return;
    }

    let mut token_table = TokenCreateTable::default();
    for controller in controllers {
        if !ctx.game.player(controller).is_alive() {
            continue;
        }

        let originals = resolve_originals(ctx, sa, controller);
        let defined = sa
            .defined()
            .filter(|_| sa.ir.choices.is_none() && sa.ir.defined_name_text.is_none());
        for original_id in originals {
            let original = match defined {
                Some(defined) => crate::ability::spell_ability_effect::defined_card_object(
                    ctx.game,
                    sa,
                    defined,
                    original_id,
                ),
                None => ctx.game.card(original_id),
            };
            if original.type_line.is_instant() || original.type_line.is_sorcery() {
                continue;
            }
            if sa.ir.defined_text.is_none()
                && sa.ir.choices.is_none()
                && sa.ir.defined_name_text.is_none()
                && ctx.game.card(original_id).zone != ZoneType::Battlefield
            {
                continue;
            }

            if let Some(for_each) = sa.ir.for_each_text.as_deref() {
                let players = crate::ability::ability_utils::resolve_defined_players_with_sa(
                    for_each,
                    sa,
                    sa.activating_player,
                    ctx.game,
                );
                for player in players {
                    let mut proto = get_proto_type(sa, original, controller);
                    proto.copied_permanent = Some(original_id);
                    proto.add_remembered_player(player);
                    token_table.put(controller, proto, amount);
                }
            } else {
                let mut proto = get_proto_type(sa, original, controller);
                proto.copied_permanent = Some(original_id);
                token_table.put(controller, proto, amount);
            }
        }
    }

    let mut trigger_list = CardZoneTable::default();
    let result = TOKEN_EFFECT_BASE.make_token_table(ctx, token_table, true, &mut trigger_list, sa);
    if !result.created.is_empty() {
        trigger_list.trigger_changes_zone_all(ctx.trigger_handler, ctx.game, Some(sa));
    }
}

/// Build the in-memory copy of `original` that a Copy/Embalm/Eternalize effect
/// will place onto the battlefield. Mirrors Java
/// `CopyPermanentEffect.getProtoType(SpellAbility, Card, Player)`.
///
/// Returned `Card` carries a placeholder `CardId(0)`; callers must invoke
/// `GameState::create_card` to receive the real id. The copy's exceptions come from
/// `card_factory::get_clone_states`, as in Java; shared token lifecycle params such as
/// `PumpKeywords` are applied by `TokenEffectBase` after the token receives its real id.
pub fn get_proto_type(sa: &SpellAbility, original: &Card, new_owner: crate::ids::PlayerId) -> Card {
    let mut copy =
        crate::card::card_factory::get_clone_states(original, new_owner, sa).into_state();
    copy.set_is_token(true);
    copy.token_spawning_ability = Some(Box::new(sa.clone()));
    copy
}

fn resolve_copy_controllers(ctx: &EffectContext, sa: &SpellAbility) -> Vec<crate::ids::PlayerId> {
    if let Some(controller) = sa.ir.controller_text.as_deref() {
        let players = crate::ability::ability_utils::resolve_defined_players_with_sa(
            controller,
            sa,
            sa.activating_player,
            ctx.game,
        );
        if !players.is_empty() {
            return players;
        }
    }
    vec![sa.activating_player]
}

fn resolve_originals(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    controller: crate::ids::PlayerId,
) -> Vec<CardId> {
    if let Some(choices) = sa.ir.choices.as_deref() {
        let candidates: Vec<CardId> = ctx
            .game
            .cards
            .iter()
            .filter(|card| card.zone == ZoneType::Battlefield)
            .map(|card| card.id)
            .filter(|&card_id| {
                let Some(source_id) = sa.source else {
                    return false;
                };
                crate::card::valid_filter::matches_valid_card_selector_with_context(
                    &crate::parsing::cached_compiled_selector(choices),
                    ctx.game.card(card_id),
                    crate::card::valid_filter::MatchContext::new(
                        ctx.game.card(source_id),
                        ctx.game,
                    )
                    .with_host_object(sa),
                )
            })
            .collect();
        if candidates.is_empty() {
            return Vec::new();
        }

        let chooser = sa
            .ir
            .chooser
            .as_deref()
            .and_then(|defined| {
                crate::ability::ability_utils::resolve_defined_players_with_sa(
                    defined,
                    sa,
                    sa.activating_player,
                    ctx.game,
                )
                .into_iter()
                .next()
            })
            .unwrap_or(sa.activating_player);
        let choices: Vec<crate::agent::GameEntity> = candidates
            .into_iter()
            .map(crate::agent::GameEntity::Card)
            .collect();
        ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
        return match ctx.agents[chooser.index()].choose_single_entity_for_effect(
            DecisionContext::new(ctx.game, ctx.mana_pools),
            chooser,
            &choices,
            false,
        ) {
            Some(crate::agent::GameEntity::Card(chosen)) => vec![chosen],
            _ => Vec::new(),
        };
    }

    if let Some(defined_name) = sa.ir.defined_name_text.as_deref() {
        let name = match (defined_name, sa.source) {
            ("NamedCard", Some(source)) => ctx
                .game
                .card(source)
                .get_named_card()
                .filter(|named| !named.is_empty())
                .unwrap_or(defined_name)
                .to_string(),
            _ => defined_name.to_string(),
        };
        let Some(rules) =
            crate::game::CardDatabaseRegistry::get().and_then(|db| db.get_by_card_name(&name))
        else {
            return Vec::new();
        };
        let card = crate::card::card_factory::from_rules(rules, controller);
        return vec![ctx.game.create_card(card)];
    }

    if let Some(defined) = sa.defined() {
        match defined {
            "Self" => return sa.source.into_iter().collect(),
            "ParentTarget" => return sa.target_chosen.target_card.into_iter().collect(),
            "TriggeredCard" | "TriggeredCardLKICopy" | "TriggeredSacrificedCard" => {
                return sa
                    .get_triggering_card(crate::ability::AbilityKey::Card)
                    .into_iter()
                    .collect();
            }
            _ => {
                return crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    ctx.game, sa, defined,
                );
            }
        }
    }

    crate::ability::spell_ability_effect::get_defined_cards_or_targeted(ctx.game, sa)
}
