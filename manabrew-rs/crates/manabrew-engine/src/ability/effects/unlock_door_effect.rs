//! UnlockDoor — unlock a door on a Room card.
//! Ported from Java's UnlockDoorEffect: unlocks one side of a Room
//! enchantment, activating its abilities.

use forge_foundation::{CardStateName, ZoneType};

use super::EffectContext;
use crate::event::RunParams;
use crate::ids::CardId;
use crate::spellability::SpellAbilityMode;
use crate::trigger::TriggerType;

fn unlocked_room_count(ctx: &EffectContext, card_id: CardId) -> i32 {
    ctx.game.card(card_id).get_unlocked_room_count()
}

/// Struct form of this effect so it can participate in the
/// `SpellAbilityEffect` trait hierarchy — mirrors Java's
/// `UnlockDoorEffect` class extending `SpellAbilityEffect`.
#[manabrew_engine_macros::spell_effect(UnlockDoorEffect)]
fn resolve(ctx: &mut EffectContext, sa: &crate::spellability::SpellAbility) {
    let targets: Vec<CardId> = if let Some(target) = sa.target_chosen.target_card {
        vec![target]
    } else if let Some(source) = sa.source {
        vec![source]
    } else {
        return;
    };

    let mode = sa.ir.mode.as_ref().unwrap_or(&SpellAbilityMode::ThisDoor);

    for card_id in targets {
        if ctx.game.card(card_id).zone != ZoneType::Battlefield {
            continue;
        }

        let before = unlocked_room_count(ctx, card_id);
        let first_locked = [CardStateName::LeftSplit, CardStateName::RightSplit]
            .into_iter()
            .find(|&state| ctx.game.card(card_id).room_door_locked(state));
        let door = sa
            .ir
            .card_state_name
            .as_deref()
            .and_then(CardStateName::from_str_compat)
            .or(match mode {
                SpellAbilityMode::Unlock => first_locked,
                SpellAbilityMode::LockOrUnlock => first_locked.or(Some(CardStateName::RightSplit)),
                _ => None,
            });
        let mut unlocked = false;
        if let Some(state) = door {
            let locked = ctx.game.card(card_id).room_door_locked(state);
            match mode {
                SpellAbilityMode::ThisDoor | SpellAbilityMode::Unlock if locked => {
                    ctx.game.card_mut(card_id).unlock_room_door(state);
                    unlocked = true;
                }
                SpellAbilityMode::LockOrUnlock => {
                    if locked {
                        ctx.game.card_mut(card_id).unlock_room_door(state);
                        unlocked = true;
                    } else {
                        ctx.game.card_mut(card_id).lock_room_door(state);
                    }
                }
                _ => {}
            }
            ctx.game.card_mut(card_id).update_rooms();
        }
        let after = unlocked_room_count(ctx, card_id);

        if unlocked {
            ctx.trigger_handler
                .register_active_trigger(ctx.game, card_id);
            ctx.trigger_handler.run_trigger(
                TriggerType::UnlockDoor,
                RunParams {
                    card: Some(card_id),
                    player: Some(sa.activating_player),
                    card_state_name: sa.ir.card_state_name.clone(),
                    ..Default::default()
                },
                true,
            );
            if before < 2 && after >= 2 {
                ctx.trigger_handler.run_trigger(
                    TriggerType::FullyUnlock,
                    RunParams {
                        card: Some(card_id),
                        player: Some(sa.activating_player),
                        ..Default::default()
                    },
                    true,
                );
            }
        }
    }
}
