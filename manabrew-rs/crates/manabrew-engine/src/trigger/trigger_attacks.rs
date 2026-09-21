use serde::{Deserialize, Serialize};

use crate::parsing::{keys, Params};
use crate::{event::RunParams, game::GameState, spellability::SpellAbility, trigger::TriggerType};

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAttacks {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    pub attacked: Option<crate::parsing::CompiledSelector>,
    pub alone: bool,
    pub first_attack: bool,
}

impl TriggerAttacks {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            attacked: params.selector_cloned("Attacked"),
            alone: params.is_true(keys::ALONE),
            first_attack: params.has("FirstAttack"),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerAttacks {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Attacks
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        if self.alone && params.num_attackers.unwrap_or(0) != 1 {
            return false;
        }
        // Java `TriggerAttacks.performTest:85` rejects once the attacker has already
        // attacked more than once this turn, so the trigger covers only its first attack.
        if self.first_attack
            && params
                .attacker
                .is_some_and(|a| game.card(a).attacks_this_turn > 1)
        {
            return false;
        }
        // Java `TriggerAttacks.performTest:64` filters on what was attacked, which is a
        // player for most triggers and a permanent for a planeswalker or battle.
        if self.attacked.is_some() {
            let matched = if params.attacked_player.is_some() {
                trigger.matches_optional_valid_player_filter(
                    &self.attacked,
                    params.attacked_player,
                    game,
                )
            } else {
                trigger.matches_optional_valid_card_filter(
                    &self.attacked,
                    params.attacked_card,
                    game,
                )
            };
            if !matched {
                return false;
            }
        }
        trigger.matches_optional_valid_card_filter(&self.valid_card, params.attacker, game)
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        // Java: sa.setTriggeringObject(AbilityKey.Defender, runParams.get(AbilityKey.Attacked));
        if let Some(p) = params.attacked_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Defender,
                crate::event::AbilityValue::Player(p),
            );
        } else if let Some(c) = params.attacked_card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Defender,
                crate::event::AbilityValue::Card(c),
            );
        }
        // Java: sa.setTriggeringObjectsFrom(runParams, AbilityKey.Attacker, AbilityKey.Defenders, AbilityKey.DefendingPlayer);
        if let Some(attacker) = params.attacker {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Attacker,
                crate::event::AbilityValue::Card(attacker),
            );
        }
        // Defenders combines both player and card defender IDs
        match (
            params.defenders_player_ids.as_ref(),
            params.defenders_card_ids.as_ref(),
        ) {
            (Some(players), None) => {
                sa.set_triggering_value(
                    crate::ability::AbilityKey::Defenders,
                    crate::event::AbilityValue::Players(players.clone()),
                );
            }
            (None, Some(cards)) => {
                sa.set_triggering_value(
                    crate::ability::AbilityKey::Defenders,
                    crate::event::AbilityValue::Cards(cards.clone()),
                );
            }
            (Some(players), Some(cards)) => {
                let objects = players
                    .iter()
                    .copied()
                    .map(crate::agent::GameEntity::Player)
                    .chain(cards.iter().copied().map(crate::agent::GameEntity::Card))
                    .collect();
                sa.set_triggering_value(
                    crate::ability::AbilityKey::Defenders,
                    crate::event::AbilityValue::GameEntities(objects),
                );
            }
            (None, None) => {}
        }
        if let Some(p) = params.defending_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::DefendingPlayer,
                crate::event::AbilityValue::Player(p),
            );
        }
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        format!(
            "Attacker: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Attacker)
                .unwrap_or_default()
        )
    }
}
