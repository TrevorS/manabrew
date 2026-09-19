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
pub struct TriggerAttackerBlockedByCreature {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    #[serde(default)]
    pub valid_card_text: Option<String>,
    #[serde(default)]
    pub valid_blocker: Option<crate::parsing::CompiledSelector>,
    #[serde(default)]
    pub valid_blocker_text: Option<String>,
}

impl TriggerAttackerBlockedByCreature {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            valid_card_text: params.get_cloned(keys::VALID_CARD),
            valid_blocker: params.selector_cloned(keys::VALID_BLOCKER),
            valid_blocker_text: params.get_cloned(keys::VALID_BLOCKER),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerAttackerBlockedByCreature {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::AttackerBlockedByCreature
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        let (Some(attacker), Some(blocker)) = (params.attacker, params.blocker) else {
            return false;
        };
        let power = |card| game.card(card).power();
        let valid_card = if self.valid_card_text.as_deref() == Some("LessPowerThanBlocker") {
            power(attacker) < power(blocker)
        } else {
            trigger.matches_optional_valid_card_filter(&self.valid_card, Some(attacker), game)
        };
        let valid_blocker = if self.valid_blocker_text.as_deref() == Some("LessPowerThanAttacker") {
            power(blocker) < power(attacker)
        } else {
            trigger.matches_optional_valid_card_filter(&self.valid_blocker, Some(blocker), game)
        };
        valid_card && valid_blocker
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        if let Some(attacker) = params.attacker {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Attacker,
                crate::event::AbilityValue::Card(attacker),
            );
        }
        if let Some(blocker) = params.blocker {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Blocker,
                crate::event::AbilityValue::Card(blocker),
            );
        }
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        format!(
            "Attacker: {}, Blocker: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Attacker)
                .unwrap_or_default(),
            sa.get_triggering_object_text(crate::ability::AbilityKey::Blocker)
                .unwrap_or_default()
        )
    }
}
