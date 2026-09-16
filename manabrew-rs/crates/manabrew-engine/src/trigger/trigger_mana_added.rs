use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerManaAdded {
    pub valid_source: Option<crate::parsing::CompiledSelector>,
    pub valid_sa: Option<String>,
    pub player: Option<crate::parsing::CompiledSelector>,
    pub produced: Option<String>,
}

impl TriggerManaAdded {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_source: params.selector_cloned("ValidSource"),
            valid_sa: params.get_cloned(keys::VALID_SA),
            player: params.selector_cloned(keys::PLAYER),
            produced: params.get_cloned(keys::PRODUCED),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerManaAdded {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::ManaAdded
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        if !trigger.matches_optional_valid_card_filter(&self.valid_source, params.card, game) {
            return false;
        }
        if let Some(filter) = self.valid_sa.as_ref() {
            let Some(sa) = params.ability_mana.as_ref() else {
                return false;
            };
            if !trigger.matches_valid_sa_filter(filter, sa) {
                return false;
            }
        }
        if !trigger.matches_optional_valid_player_filter(&self.player, params.player, game) {
            return false;
        }
        if let Some(expected) = self.produced.as_ref() {
            let Some(actual) = params.produced.as_ref() else {
                return false;
            };
            if !actual.contains(expected) {
                return false;
            }
        }
        true
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
        if let Some(p) = params.player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Player,
                crate::event::AbilityValue::Player(p),
            );
        }
        if let Some(produced) = params.produced.as_ref() {
            sa.set_triggering_object(crate::ability::AbilityKey::Produced, produced);
        }
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        format!(
            "Produced: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Produced)
                .unwrap_or_default()
        )
    }
}
