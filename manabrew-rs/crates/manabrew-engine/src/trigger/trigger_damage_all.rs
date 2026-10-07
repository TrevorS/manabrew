use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDamageAll {
    pub valid_source: Option<crate::parsing::CompiledSelector>,
    pub valid_target: Option<crate::parsing::CompiledSelector>,
    #[serde(default)]
    pub combat_damage: Option<bool>,
}

impl TriggerDamageAll {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_source: params.selector_cloned(keys::VALID_SOURCE),
            valid_target: params.selector_cloned(keys::VALID_TARGET),
            combat_damage: params
                .get(keys::COMBAT_DAMAGE)
                .map(|v| v.eq_ignore_ascii_case("True")),
        })
    }

    fn filtered_map(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> crate::card::card_damage_map::CardDamageMap {
        let valid_source = self.valid_source.as_ref().map(|s| s.as_raw());
        let valid_target = self.valid_target.as_ref().map(|s| s.as_raw());
        params
            .damage_map
            .as_ref()
            .map(|map| {
                map.filtered_map(
                    game,
                    valid_source.as_deref(),
                    valid_target.as_deref(),
                    trigger.host_card_id(),
                )
            })
            .unwrap_or_default()
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerDamageAll {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::DamageAll
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        if let Some(wants_combat) = self.combat_damage {
            if params.is_combat_damage.unwrap_or(false) != wants_combat {
                return false;
            }
        }
        !self
            .filtered_map(trigger, params, game)
            .entries()
            .is_empty()
    }

    fn set_triggering_objects(
        &self,
        trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        game: &GameState,
    ) {
        let table = self.filtered_map(trigger, params, game);
        sa.set_triggering_object(
            crate::ability::AbilityKey::DamageAmount,
            table.total_amount().to_string(),
        );
        let mut sources = Vec::new();
        let mut targets = Vec::new();
        for (source, target, _) in table.entries() {
            if !sources.contains(&source) {
                sources.push(source);
            }
            let entity = match target {
                crate::card::card_damage_map::DamageTarget::Card(card) => {
                    crate::agent::GameEntity::Card(card)
                }
                crate::card::card_damage_map::DamageTarget::Player(player) => {
                    crate::agent::GameEntity::Player(player)
                }
            };
            if !targets.contains(&entity) {
                targets.push(entity);
            }
        }
        sa.set_triggering_value(
            crate::ability::AbilityKey::Sources,
            crate::event::AbilityValue::Cards(sources),
        );
        sa.set_triggering_value(
            crate::ability::AbilityKey::Targets,
            crate::event::AbilityValue::GameEntities(targets),
        );
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        // Java: "Damage Source: " + Sources + ", Damaged: " + Targets + ", Amount: " + DamageAmount
        format!(
            "Damage Source: {}, Damaged: {}, Amount: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Sources)
                .unwrap_or_default(),
            sa.get_triggering_object_text(crate::ability::AbilityKey::Targets)
                .unwrap_or_default(),
            sa.get_triggering_object(crate::ability::AbilityKey::DamageAmount)
                .unwrap_or("")
        )
    }
}
