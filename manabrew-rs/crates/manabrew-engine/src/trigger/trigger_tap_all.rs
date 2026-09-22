use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerTapAll {
    pub valid_card: Option<crate::parsing::CompiledSelector>,
}

impl TriggerTapAll {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        // Java's `TriggerTapAll` reads `ValidCards$` (plural — it filters the whole batch of
        // cards that became tapped), not the singular `ValidCard$` most other triggers use.
        Box::new(Self {
            valid_card: params.selector_cloned(keys::VALID_CARDS),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerTapAll {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::TapAll
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        // Java's `TriggerTapAll.performTest` is `matchesValidParam("ValidCards",
        // runParams.get(Cards))` — the whole batch of cards that became tapped together, not
        // a single `Card` payload. `AbilityKey.Cards` is what the batch tap sites (combat
        // declaring attackers) set.
        let Some(cards) = params.cards.as_ref() else {
            return false;
        };
        cards.iter().any(|&card_id| {
            trigger.matches_optional_valid_card_filter(&self.valid_card, Some(card_id), game)
        })
    }

    fn set_triggering_objects(
        &self,
        trigger: &Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        game: &GameState,
    ) {
        // Java's `setTriggeringObjects` filters the batch down to the cards `ValidCards$`
        // actually accepts (`IterableUtil.filter`) before handing it to the executed ability —
        // the untapped ones in `params.cards` may include cards the trigger's own test ignored.
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
            sa.set_triggering_value(
                crate::ability::AbilityKey::Cards,
                crate::event::AbilityValue::Cards(filtered),
            );
        }
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        format!(
            "Tapped: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Cards)
                .unwrap_or_default()
        )
    }
}
