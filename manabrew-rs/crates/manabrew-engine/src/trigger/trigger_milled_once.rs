use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerMilledOnce {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    pub valid_player: Option<crate::parsing::CompiledSelector>,
}

impl TriggerMilledOnce {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARD),
            valid_player: params.selector_cloned(keys::VALID_PLAYER),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerMilledOnce {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::MilledOnce
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        let _host_card = trigger.base.card_trait_base.host_card_id();
        let _host_controller = trigger.base.card_trait_base.host_controller(game);
        if !trigger.matches_optional_valid_player_filter(&self.valid_player, params.player, game) {
            return false;
        }
        let Some(cards) = params.cards.as_ref() else {
            return false;
        };
        cards.iter().any(|&cid| {
            trigger.matches_optional_valid_card_filter(&self.valid_card, Some(cid), game)
        })
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        // TODO: port ValidCard filtering from Java (CardLists.getValidCards)
        if let Some(cards) = params.cards.as_ref() {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Cards,
                crate::event::AbilityValue::Cards(cards.clone()),
            );
            sa.set_triggering_object(crate::ability::AbilityKey::Amount, cards.len().to_string());
        }
        if let Some(p) = params.player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Player,
                crate::event::AbilityValue::Player(p),
            );
        }
    }

    fn get_important_stack_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &SpellAbility,
    ) -> String {
        format!(
            "Player: {}",
            sa.trigger_objects
                .get(&crate::ability::AbilityKey::Player)
                .map(|s| s.as_str())
                .unwrap_or("")
        )
    }
}
