use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDiscardedAll {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    pub valid_player: Option<crate::parsing::CompiledSelector>,
}

impl TriggerDiscardedAll {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            valid_player: params.selector_cloned(keys::VALID_PLAYER),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerDiscardedAll {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::DiscardedAll
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        let _host_card = trigger.base.card_trait_base.host_card_id();
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        // Java's key here is singular `ValidCard`, unlike TapAll's plural `ValidCards`, and it
        // is matched against the whole `Cards` batch.
        let cards_match = match params.cards.as_ref() {
            Some(cards) => cards.iter().any(|&card_id| {
                trigger.matches_optional_valid_card_filter(&self.valid_card, Some(card_id), game)
            }),
            None => self.valid_card.is_none(),
        };
        cards_match
            && trigger.matches_optional_valid_player_filter(&self.valid_player, params.player, game)
    }

    fn set_triggering_objects(
        &self,
        trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        game: &GameState,
    ) {
        if let Some(cards) = params.cards.as_ref() {
            let filtered: Vec<_> = cards
                .iter()
                .copied()
                .filter(|&card_id| {
                    trigger.matches_optional_valid_card_filter(
                        &self.valid_card,
                        Some(card_id),
                        game,
                    )
                })
                .collect();
            sa.set_triggering_object(
                crate::ability::AbilityKey::Amount,
                filtered.len().to_string(),
            );
            sa.set_triggering_value(
                crate::ability::AbilityKey::Cards,
                crate::event::AbilityValue::Cards(filtered),
            );
        }
        if let Some(p) = params.player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Player,
                crate::event::AbilityValue::Player(p),
            );
        }
        // TODO: AbilityKey.Cause is a SpellAbility in Java, cannot be stored as String easily
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        format!(
            "Player: {}, Amount: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Player)
                .unwrap_or_default(),
            sa.get_triggering_object(crate::ability::AbilityKey::Amount)
                .unwrap_or_default()
        )
    }
}
