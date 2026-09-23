use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerTaps {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    pub valid_cause: Option<crate::parsing::CompiledSelector>,
    pub valid_player: Option<crate::parsing::CompiledSelector>,
    pub attacker: Option<bool>,
    pub require_first_time: bool,
    pub require_teamwork: bool,
}

impl TriggerTaps {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            valid_cause: params.selector_cloned(keys::VALID_CAUSE),
            valid_player: params.selector_cloned(keys::VALID_PLAYER),
            attacker: params
                .get(keys::ATTACKER)
                .map(|v| v.eq_ignore_ascii_case("true")),
            require_first_time: params.has("FirstTime"),
            require_teamwork: params.has("Teamwork"),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerTaps {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::Taps
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        if !trigger.matches_optional_valid_card_filter(&self.valid_card, params.card, game) {
            return false;
        }
        if let Some(filter) = self.valid_cause.as_ref() {
            let Some(cause_sa) = params.cause.as_ref() else {
                return false;
            };
            let Some(cause_card) = cause_sa.source else {
                return false;
            };
            if !trigger.matches_valid_card_filter(filter, cause_card, game) {
                return false;
            }
        }
        if !trigger.matches_optional_valid_player_filter(&self.valid_player, params.player, game) {
            return false;
        }
        if let Some(expected_attacker) = self.attacker {
            if params.attacker.is_some() != expected_attacker {
                return false;
            }
        }
        if self.require_first_time
            && !params.first_time.unwrap_or_else(|| {
                params
                    .card
                    .is_some_and(|card| game.card(card).tapped_this_turn == 1)
            })
        {
            return false;
        }
        // Java `TriggerTaps` requires the running cost payment to be a `CostTeamwork`;
        // this port has no Teamwork cost part, so that test can never pass.
        if self.require_teamwork {
            return false;
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
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        format!(
            "Tapped: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Card)
                .unwrap_or_default()
        )
    }
}
