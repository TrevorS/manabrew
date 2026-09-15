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
    pub valid_card: Option<crate::parsing::CompiledSelector>,
}

impl TriggerAttached {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
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
        let _host_card = trigger.base.card_trait_base.host_card_id();
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        trigger.matches_optional_valid_card_filter(&self.valid_card, params.card, game)
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
