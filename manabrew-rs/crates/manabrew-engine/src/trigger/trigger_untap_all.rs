use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerUntapAll {
    pub valid_player: Option<crate::parsing::CompiledSelector>,
    pub valid_cards: Option<crate::parsing::CompiledSelector>,
}

impl TriggerUntapAll {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_player: params.selector_cloned(keys::VALID_PLAYER),
            valid_cards: params.selector_cloned(keys::VALID_CARDS),
        })
    }

    fn filtered_map(
        &self,
        trigger: &Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> Vec<(crate::ids::PlayerId, Vec<crate::ids::CardId>)> {
        let mut pass_map = Vec::new();
        for (player, cards) in params.map.iter().flatten() {
            if trigger.matches_optional_valid_player_filter(&self.valid_player, Some(*player), game)
            {
                let mut pass_cards = Vec::new();
                if self.valid_cards.is_some() {
                    for card in cards {
                        if trigger.matches_optional_valid_card_filter(
                            &self.valid_cards,
                            Some(*card),
                            game,
                        ) {
                            pass_cards.push(*card);
                        }
                    }
                }
                if !pass_cards.is_empty() {
                    pass_map.push((*player, pass_cards));
                }
            }
        }
        pass_map
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerUntapAll {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::UntapAll
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        !self.filtered_map(trigger, params, game).is_empty()
    }

    fn set_triggering_objects(
        &self,
        trigger: &Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        game: &GameState,
    ) {
        let map = self.filtered_map(trigger, params, game);
        let untapped: Vec<crate::ids::CardId> = map
            .iter()
            .flat_map(|(_, cards)| cards.iter().copied())
            .collect();
        if let Some((player, _)) = map.first() {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Player,
                crate::event::AbilityValue::Player(*player),
            );
        }
        let csv = untapped
            .iter()
            .map(|c| c.0.to_string())
            .collect::<Vec<_>>()
            .join(",");
        sa.set_triggering_object(crate::ability::AbilityKey::Cards, &csv);
        sa.set_triggering_object(
            crate::ability::AbilityKey::Amount,
            untapped.len().to_string(),
        );
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        format!(
            "Amount: {}",
            sa.trigger_objects
                .get(&crate::ability::AbilityKey::Amount)
                .map(|s| s.as_str())
                .unwrap_or("")
        )
    }
}
