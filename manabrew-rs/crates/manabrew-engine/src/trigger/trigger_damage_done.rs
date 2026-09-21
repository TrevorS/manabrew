use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDamageDone {
    pub valid_source: Option<crate::parsing::CompiledSelector>,
    pub valid_target: Option<crate::parsing::CompiledSelector>,
    pub combat_damage: Option<bool>,
}

impl TriggerDamageDone {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_source: params.selector_cloned(keys::VALID_SOURCE),
            valid_target: params.selector_cloned(keys::VALID_TARGET),
            combat_damage: params
                .get(keys::COMBAT_DAMAGE)
                .map(|v| v.eq_ignore_ascii_case("True")),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerDamageDone {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::DamageDone
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        // Java compares the parameter against the run param for True and False alike
        // (`TriggerDamageDone.performTest`), so `CombatDamage$ False` excludes combat damage.
        if let Some(wants_combat) = self.combat_damage {
            if params.is_combat_damage.unwrap_or(false) != wants_combat {
                return false;
            }
        }
        trigger.matches_optional_valid_card_filter(&self.valid_source, params.damage_source, game)
            && trigger.matches_damage_target_filter(&self.valid_target, params, game, true)
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        // Java: sa.setTriggeringObject(AbilityKey.Source, CardCopyService.getLKICopy(DamageSource))
        // TODO: Java uses CardCopyService.getLKICopy for the source. We just use the ID directly.
        if let Some(src) = params.damage_source {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Source,
                crate::event::AbilityValue::Card(src),
            );
        }
        if let Some(card) = params.damage_target_card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Target,
                crate::event::AbilityValue::Card(card),
            );
            sa.set_triggering_value(
                crate::ability::AbilityKey::TargetCard,
                crate::event::AbilityValue::Card(card),
            );
        } else if let Some(player) = params.damage_target_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Target,
                crate::event::AbilityValue::Player(player),
            );
            sa.set_triggering_value(
                crate::ability::AbilityKey::TargetPlayer,
                crate::event::AbilityValue::Player(player),
            );
        }
        // TODO: Java also sets Cause (SpellAbility) from runParams.
        // Skipping Cause for now since SpellAbility is complex and stored as object in Java.
        if let Some(amount) = params.damage_amount {
            sa.set_triggering_object(crate::ability::AbilityKey::DamageAmount, amount.to_string());
        }
        if let Some(p) = params.defending_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::DefendingPlayer,
                crate::event::AbilityValue::Player(p),
            );
        }
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        // Java: "Damage Source: " + Source + ", Damaged: " + Target + ", Amount: " + DamageAmount
        format!(
            "Damage Source: {}, Damaged: {}, Amount: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Source)
                .unwrap_or_default(),
            sa.get_triggering_object_text(crate::ability::AbilityKey::Target)
                .unwrap_or_default(),
            sa.get_triggering_object(crate::ability::AbilityKey::DamageAmount)
                .unwrap_or("")
        )
    }
}
