use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerPlanarDice {
    pub valid_player: Option<crate::parsing::CompiledSelector>,
    pub result: Option<String>,
}

impl TriggerPlanarDice {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_player: params.selector_cloned(keys::VALID_PLAYER),
            result: params.get_cloned("Result"),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerPlanarDice {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::PlanarDice
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        if !trigger.matches_optional_valid_player_filter(&self.valid_player, params.player, game) {
            return false;
        }
        if let Some(expected) = self.result.as_ref() {
            return params.mode.as_ref() == Some(expected);
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
        if let Some(p) = params.player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Player,
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
            "Roller: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Player)
                .unwrap_or_default()
        )
    }
}
