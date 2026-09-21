use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerLandPlayed {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    pub origin: Option<String>,
}

impl TriggerLandPlayed {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            origin: params.get(keys::ORIGIN).map(str::to_string),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerLandPlayed {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::LandPlayed
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        let _host_card = trigger.base.card_trait_base.host_card_id();
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        // Java `TriggerLandPlayed.performTest` filters on the zone the land was played
        // from, so a trigger that wants it played from anywhere but hand says so here.
        if let Some(origin) = self.origin.as_deref() {
            if origin != "Any" {
                let Some(played_from) = params.origin else {
                    return false;
                };
                if !origin
                    .split(',')
                    .filter_map(|z| forge_foundation::ZoneType::from_str_compat(z.trim()))
                    .any(|z| z == played_from)
                {
                    return false;
                }
            }
        }
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
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        // Java: "LandPlayed: " + Card
        format!(
            "LandPlayed: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Card)
                .unwrap_or_default()
        )
    }
}
