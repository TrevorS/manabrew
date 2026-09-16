use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerCounterTypeAddedAll {
    pub valid_object: Option<crate::parsing::CompiledSelector>,
    pub first_time_only: bool,
}

impl TriggerCounterTypeAddedAll {
    pub fn parse(
        valid_object: Option<crate::parsing::CompiledSelector>,
        first_time_only: bool,
    ) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_object,
            first_time_only,
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerCounterTypeAddedAll {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::CounterTypeAddedAll
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        if let Some(filter) = &self.valid_object {
            let object_ok = if let Some(cid) = params.object_card.or(params.card) {
                trigger.matches_valid_card_filter(filter, cid, game)
            } else if let Some(pid) = params.object_player.or(params.player) {
                trigger.matches_valid_player_filter(filter, pid, game)
            } else {
                false
            };
            if !object_ok {
                return false;
            }
        }

        if self.first_time_only && params.first_time != Some(true) {
            return false;
        }

        true
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        if let Some(obj) = params.object_card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Object,
                crate::event::AbilityValue::Card(obj),
            );
        } else if let Some(p) = params.object_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Object,
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
            "AddedOnce: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Object)
                .unwrap_or_default()
        )
    }
}
