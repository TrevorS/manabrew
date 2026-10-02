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
    pub valid_cause: Option<String>,
}

impl TriggerTurnFaceUp {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            valid_cause: params.get(keys::VALID_CAUSE).map(str::to_string),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerTurnFaceUp {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::TurnFaceUp
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        if !trigger.matches_optional_valid_card_filter(&self.valid_card, params.card, game) {
            return false;
        }
        self.valid_cause.as_deref().is_none_or(|filter| {
            params.cause.as_ref().is_some_and(|cause| {
                crate::spellability::matches_valid_sa(
                    filter,
                    cause,
                    cause.source.map(|id| game.card(id)),
                    crate::card::valid_filter::MatchContext::new(
                        game.card(trigger.host_card_id()),
                        game,
                    ),
                )
            })
        })
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
