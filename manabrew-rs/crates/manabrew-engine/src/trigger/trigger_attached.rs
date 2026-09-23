use serde::{Deserialize, Serialize};

use crate::{
    event::RunParams,
    game::GameState,
    parsing::{keys, Params},
    spellability::SpellAbility,
    trigger::TriggerType,
};

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerAttached {
    pub valid_source: Option<crate::parsing::CompiledSelector>,
    pub valid_target: Option<crate::parsing::CompiledSelector>,
}

impl TriggerAttached {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_source: params.selector_cloned(keys::VALID_SOURCE),
            valid_target: params.selector_cloned(keys::VALID_TARGET),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerAttached {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Attached
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        trigger.matches_optional_valid_card_filter(&self.valid_source, params.source_card, game)
            && trigger.matches_optional_valid_card_filter(&self.valid_target, params.card, game)
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        if let Some(source) = params.source_card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Source,
                crate::event::AbilityValue::Card(source),
            );
        }
        if let Some(card) = params.card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Target,
                crate::event::AbilityValue::Card(card),
            );
        }
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        format!(
            "Attachee: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Target)
                .unwrap_or_default()
        )
    }
}
