use crate::HashSet;

use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::ids::CardId;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDamageDoneOnce {
    pub valid_source: Option<crate::parsing::CompiledSelector>,
    pub valid_target: Option<crate::parsing::CompiledSelector>,
    pub combat_damage: Option<bool>,
    pub damage_amount_text: Option<String>,
}

impl TriggerDamageDoneOnce {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_source: params.selector_cloned(keys::VALID_SOURCE),
            valid_target: params.selector_cloned(keys::VALID_TARGET),
            combat_damage: params
                .get(keys::COMBAT_DAMAGE)
                .map(|v| v.eq_ignore_ascii_case("True")),
            damage_amount_text: params.get(keys::DAMAGE_AMOUNT).map(str::to_string),
        })
    }

    fn damage_amount(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> i32 {
        if let Some(map) = params.damage_map.as_ref() {
            return map
                .entries()
                .into_iter()
                .filter(|(source, _, _)| {
                    trigger.matches_optional_valid_card_filter(
                        &self.valid_source,
                        Some(*source),
                        game,
                    )
                })
                .map(|(_, _, amount)| amount)
                .sum();
        }
        if self.valid_source.is_some()
            && !trigger.matches_optional_valid_card_filter(
                &self.valid_source,
                params.damage_source,
                game,
            )
        {
            return 0;
        }
        params.damage_amount.unwrap_or(0)
    }

    fn damage_sources(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> Vec<CardId> {
        if let Some(map) = params.damage_map.as_ref() {
            let mut seen = HashSet::default();
            let mut sources = Vec::new();
            for (source, _, _) in map.entries() {
                if !trigger.matches_optional_valid_card_filter(
                    &self.valid_source,
                    Some(source),
                    game,
                ) {
                    continue;
                }
                if seen.insert(source) {
                    sources.push(source);
                }
            }
            return sources;
        }
        params.damage_source.into_iter().collect()
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerDamageDoneOnce {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::DamageDoneOnce
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
        if !trigger.matches_damage_target_filter(&self.valid_target, params, game, true) {
            return false;
        }
        let dealt = self.damage_amount(trigger, params, game);
        // Java compares the whole amount against `DamageAmount$` (`TriggerDamageDoneOnce:38`),
        // so a trigger that wants three or more does not fire on one.
        if let Some(amount) = self.damage_amount_text.as_deref() {
            let operator = amount.get(..2).unwrap_or("GE");
            let operand = amount.get(2..).unwrap_or("0");
            if !crate::parsing::compare::compare_expr(dealt, &format!("{operator}{operand}")) {
                return false;
            }
        }
        dealt > 0
    }

    fn set_triggering_objects(
        &self,
        trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        game: &GameState,
    ) {
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
        let sources = self.damage_sources(trigger, params, game);
        if !sources.is_empty() {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Sources,
                crate::event::AbilityValue::Cards(sources),
            );
        }
        if let Some(p) = params.attacking_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::AttackingPlayer,
                crate::event::AbilityValue::Player(p),
            );
        }
        let amount = self.damage_amount(trigger, params, game);
        sa.set_triggering_object(crate::ability::AbilityKey::DamageAmount, amount.to_string());
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        // Java: if Target != null { "Damaged: " + Target + ", " } + "Amount: " + DamageAmount
        let target = sa
            .get_triggering_object_text(crate::ability::AbilityKey::Target)
            .unwrap_or_default();
        if target.is_empty() {
            format!(
                "Amount: {}",
                sa.get_triggering_object(crate::ability::AbilityKey::DamageAmount)
                    .unwrap_or("")
            )
        } else {
            format!(
                "Damaged: {}, Amount: {}",
                target,
                sa.get_triggering_object(crate::ability::AbilityKey::DamageAmount)
                    .unwrap_or("")
            )
        }
    }
}

/// Returns the total damage amount from the damage map.
/// Java: TriggerDamageDoneOnce.getDamageAmount
///
/// Note: The Java version filters entries by ValidSource param; this standalone
/// function passes all entries through. Filtering will be added when trigger
/// param context is available.
pub fn get_damage_amount(params: &RunParams) -> i32 {
    match params.damage_map.as_ref() {
        Some(map) => map.total_amount(),
        None => 0,
    }
}

/// Returns the damage source card IDs from the damage map.
/// Java: TriggerDamageDoneOnce.getDamageSources
///
/// Note: The Java version filters entries by ValidSource param; this standalone
/// function returns all source card IDs. Filtering will be added when trigger
/// param context is available.
pub fn get_damage_sources(params: &RunParams) -> Vec<CardId> {
    match params.damage_map.as_ref() {
        Some(map) => {
            let mut seen = HashSet::default();
            let mut sources = Vec::new();
            for (source, _, _) in map.entries() {
                if seen.insert(source) {
                    sources.push(source);
                }
            }
            sources
        }
        None => Vec::new(),
    }
}
