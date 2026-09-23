use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::ids::CardId;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerExcessDamageAll {
    pub valid_target: Option<crate::parsing::CompiledSelector>,
    pub combat_damage: Option<bool>,
}

impl TriggerExcessDamageAll {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_target: params.selector_cloned(keys::VALID_TARGET),
            combat_damage: params
                .get(keys::COMBAT_DAMAGE)
                .map(|value| value.eq_ignore_ascii_case("True")),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerExcessDamageAll {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::ExcessDamageAll
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        if let Some(combat_damage) = self.combat_damage {
            if params.is_combat_damage != Some(combat_damage) {
                return false;
            }
        }

        let targets = params.cards.as_deref().unwrap_or(&[]);
        if targets.is_empty() {
            return false;
        }
        if self.valid_target.is_some() {
            return targets.iter().any(|&cid| {
                trigger.matches_optional_valid_card_filter(&self.valid_target, Some(cid), game)
            });
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
        // Java: sa.setTriggeringObject(AbilityKey.Targets, getDamageTargets(DamageTargets))
        // TODO: getDamageTargets filters with ValidTarget — free function has no access to trigger params
        if let Some(cards) = params.cards.as_ref() {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Targets,
                crate::event::AbilityValue::Cards(cards.clone()),
            );
        }
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        // Java: "Damaged: " + Targets
        format!(
            "Damaged: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Targets)
                .unwrap_or_default()
        )
    }
}

/// Returns the damage target card IDs from the cards list in RunParams.
/// Java: TriggerExcessDamageAll.getDamageTargets
///
/// Note: The Java version filters entries by ValidTarget param; this standalone
/// function returns all target card IDs. Filtering will be added when trigger
/// param context is available.
pub fn get_damage_targets(params: &RunParams) -> Vec<CardId> {
    params.cards.clone().unwrap_or_default()
}
