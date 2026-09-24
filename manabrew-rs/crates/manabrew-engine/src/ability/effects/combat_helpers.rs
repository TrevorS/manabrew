//! Combat-side helpers shared by effect resolvers.
//!
//! Mirrors Java's `SpellAbilityEffect.addToCombat` + defender-selection logic
//! used by `AttachEffect`, `AnimateEffect`, and `SetStateEffect`.

use forge_foundation::CoreType;

use crate::agent::GameEntity;
use crate::combat::DefenderId;
use crate::ids::CardId;
use crate::spellability::SpellAbility;

use super::effect_context::EffectContext;

pub(super) fn choose_defender(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    defenders: &[DefenderId],
) -> Option<DefenderId> {
    if defenders.is_empty() {
        return None;
    }
    let chooser = sa.activating_player;
    let choices: Vec<GameEntity> = defenders
        .iter()
        .map(|defender| match defender {
            DefenderId::Player(pid) => GameEntity::Player(*pid),
            DefenderId::Permanent(cid) => GameEntity::Card(*cid),
        })
        .collect();
    ctx.agents[chooser.index()].snapshot_state(ctx.game, ctx.mana_pools);
    match ctx.agents[chooser.index()].choose_single_entity_for_effect(chooser, &choices, false)? {
        GameEntity::Player(pid) => Some(DefenderId::Player(pid)),
        GameEntity::Card(cid) => Some(DefenderId::Permanent(cid)),
    }
}

pub(super) fn resolve_attack_defenders(
    ctx: &EffectContext,
    sa: &SpellAbility,
    card_id: CardId,
    attacking_param: &str,
) -> Vec<DefenderId> {
    let controller = ctx.game.card(card_id).controller;
    let possible = crate::combat::get_possible_defenders(ctx.game, controller);
    if attacking_param.eq_ignore_ascii_case("True") {
        return possible;
    }

    if attacking_param.eq_ignore_ascii_case("TriggeredDefender") {
        let mut defenders = Vec::new();
        if let Some(pid) = sa.get_triggering_player(crate::ability::AbilityKey::Defender) {
            defenders.push(DefenderId::Player(pid));
        }
        if defenders.is_empty() {
            if let Some(cid) = sa.get_triggering_card(crate::ability::AbilityKey::Defender) {
                defenders.push(DefenderId::Permanent(cid));
            }
        }
        if defenders.is_empty() {
            if let Some(pid) = sa.get_triggering_player(crate::ability::AbilityKey::DefendingPlayer)
            {
                defenders.push(DefenderId::Player(pid));
            }
        }
        if defenders.is_empty() {
            if let Some(pid) = sa.get_triggering_player(crate::ability::AbilityKey::AttackedTarget)
            {
                defenders.push(DefenderId::Player(pid));
            }
        }
        if defenders.is_empty() {
            defenders.extend(
                sa.get_triggering_cards(crate::ability::AbilityKey::Attacked)
                    .into_iter()
                    .map(DefenderId::Permanent),
            );
        }
        defenders.retain(|defender| possible.contains(defender));
        return defenders;
    }

    let mut defenders: Vec<DefenderId> =
        crate::ability::ability_utils::resolve_defined_players_with_sa(
            attacking_param,
            sa,
            sa.activating_player,
            ctx.game,
        )
        .into_iter()
        .map(DefenderId::Player)
        .collect();

    if defenders.is_empty() {
        defenders.extend(
            sa.get_triggering_cards(crate::ability::AbilityKey::Attacked)
                .into_iter()
                .map(DefenderId::Permanent),
        );
    }

    defenders.retain(|defender| possible.contains(defender));
    defenders
}

pub(crate) fn add_to_combat(
    ctx: &mut EffectContext,
    sa: &SpellAbility,
    card_id: CardId,
    attacking_param: &str,
) -> bool {
    if !ctx.game.turn.is_combat() || !ctx.game.card(card_id).is_creature() {
        return false;
    }

    let controller = ctx.game.card(card_id).controller;

    let attacking = match attacking_param {
        crate::parsing::keys::ATTACKING => sa.ir.attacking_text.as_deref(),
        crate::parsing::keys::NINJUTSU => sa.ir.ninjutsu_text.as_deref(),
        crate::parsing::keys::TOKEN_ATTACKING => sa.ir.token_attacking_text.as_deref(),
        _ => None,
    };
    let Some(attacking) = attacking else {
        return false;
    };
    if ctx
        .combat
        .as_deref()
        .and_then(|combat| combat.attacking_player)
        != Some(controller)
    {
        return false;
    }
    let defenders = resolve_attack_defenders(ctx, sa, card_id, attacking);
    let Some(defender) = choose_defender(ctx, sa, &defenders) else {
        return false;
    };

    let Some(combat) = ctx.combat.as_deref_mut() else {
        return false;
    };

    if combat
        .attackers
        .iter()
        .any(|&(attacker, current)| attacker == card_id && current == defender)
    {
        return false;
    }

    combat.remove_from_combat(card_id, ctx.game);
    combat.add_attacker(card_id, defender, ctx.game.card(card_id).zone_timestamp);

    let defending_player = defender.controlling_player(ctx.game);
    let tracked_defender = match defender {
        DefenderId::Player(pid) => crate::card::card_damage_history::TrackedEntity::Player(pid),
        DefenderId::Permanent(cid) => crate::card::card_damage_history::TrackedEntity::Card(cid),
    };
    let num_other_attackers = combat.attackers.len().saturating_sub(1) as i32;
    let defender_is_battle = matches!(
        defender,
        DefenderId::Permanent(cid) if ctx.game.card(cid).type_line.core_types.contains(&CoreType::Battle)
    );

    let card = ctx.game.card_mut(card_id);
    card.set_attacking_player(defending_player);
    card.mark_attacked_this_turn();
    card.damage_history.set_creature_attacked_this_combat(
        Some(tracked_defender),
        num_other_attackers,
        defender_is_battle,
    );
    true
}
