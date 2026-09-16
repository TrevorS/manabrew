use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerTurnFaceUp {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
}

impl TriggerTurnFaceUp {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerTurnFaceUp {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::TurnFaceUp
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        let _host_card = trigger.base.card_trait_base.host_card_id();
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        trigger.matches_optional_valid_card_filter(&self.valid_card, params.card, game)
    }

    fn set_triggering_objects(
        &self,
        _trigger: &Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        if let Some(card) = params.card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Card,
                crate::event::AbilityValue::Card(card),
            );
        }
        // TODO: port SpellAbility triggering object (AbilityKey.Cause)
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        format!(
            "TurnFaceUp: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Card)
                .unwrap_or_default()
        )
    }
}
