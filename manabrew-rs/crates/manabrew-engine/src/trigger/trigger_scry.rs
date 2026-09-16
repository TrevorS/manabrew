use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerScry {
    pub valid_player: Option<crate::parsing::CompiledSelector>,
}

impl TriggerScry {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_player: params.selector_cloned(keys::VALID_PLAYER),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerScry {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Scry
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        trigger.matches_optional_valid_player_filter(&self.valid_player, params.player, game)
    }

    fn set_triggering_objects(
        &self,
        _trigger: &Trigger,
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
        if let Some(n) = params.num {
            sa.set_triggering_object(crate::ability::AbilityKey::ScryNum, n.to_string());
        }
        // TODO: port ScryBottom triggering object (AbilityKey.ScryBottom) - field not yet in RunParams
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        format!(
            "Scryer: {}, {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Player)
                .unwrap_or_default(),
            sa.get_triggering_object_text(crate::ability::AbilityKey::ScryNum)
                .unwrap_or_default()
        )
    }
}
