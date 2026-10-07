use serde::{Deserialize, Serialize};

use crate::card::valid_filter;
use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::TriggerBehavior;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerBecomesTarget {
    pub valid_source: Option<crate::parsing::CompiledSelector>,
    pub valid_target: Option<crate::parsing::CompiledSelector>,
    pub require_first_time: bool,
    pub require_valiant: bool,
}

impl TriggerBecomesTarget {
    pub fn parse(params: &Params) -> Box<dyn TriggerBehavior> {
        // Java parity: BecomesTarget triggers can use either `ValidTarget$` or
        // `ValidCard$` to filter which card-becoming-target counts. Keyword-generated
        // Ward uses `ValidCard$ Card.Self` (see keyword_gen.rs); other scripts use
        // `ValidTarget$`. Without the `ValidCard` fallback Ward fires on *every*
        // BecomesTarget event in the game, since both filters end up None.
        let valid_target = params
            .selector_cloned(keys::VALID_TARGET)
            .or_else(|| params.selector_cloned(keys::VALID_CARD));
        Box::new(Self {
            valid_source: params.selector_cloned(keys::VALID_SOURCE),
            valid_target,
            require_first_time: params.has("FirstTime"),
            require_valiant: params.has("Valiant"),
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerBecomesTarget {
    fn trigger_type(&self) -> TriggerType {
        TriggerType::BecomesTarget
    }

    fn perform_test(
        &self,
        trigger: &super::trigger::Trigger,
        params: &RunParams,
        game: &GameState,
    ) -> bool {
        let host_controller = trigger.base.card_trait_base.host_controller(game);
        if let Some(filter) = self.valid_source.as_ref() {
            let source_matches = if let Some(source_sa) = params.source_sa.as_ref() {
                crate::spellability::matches_valid_sa(
                    &filter.as_raw(),
                    source_sa,
                    source_sa.source.map(|card| game.card(card)),
                    crate::card::valid_filter::MatchContext::new(
                        game.card(trigger.host_card_id()),
                        game,
                    )
                    .with_source_controller(host_controller),
                )
            } else if let Some(source_card) = params.cause_card {
                trigger.matches_valid_card_filter(filter, source_card, game)
            } else {
                false
            };
            if !source_matches {
                return false;
            }
        }

        if let Some(filter) = self.valid_target.as_ref() {
            let target_card = params.target_card.or(params.card);
            let target_player = params.target_player.or(params.player);
            let host = game.card(trigger.host_card_id());
            if let Some(target_sa) = params.target_sa.as_ref() {
                if !crate::spellability::matches_valid_sa(
                    &filter.as_raw(),
                    target_sa,
                    target_sa.source.map(|card| game.card(card)),
                    crate::card::valid_filter::MatchContext::new(host, game),
                ) {
                    return false;
                }
            } else if !valid_filter::matches_valid(
                &filter.as_raw(),
                target_card.map(|id| game.card(id)),
                target_player,
                host,
                host_controller,
                game,
            ) {
                return false;
            }
        }

        if self.require_first_time && params.first_time != Some(true) {
            return false;
        }
        if self.require_valiant && params.valiant != Some(true) {
            return false;
        }
        true
    }

    fn set_triggering_objects(
        &self,
        _trigger: &super::trigger::Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        if let Some(source_sa) = params.source_sa.as_deref() {
            if let Some(source_card) = source_sa.source {
                sa.set_triggering_value(
                    crate::ability::AbilityKey::Source,
                    crate::event::AbilityValue::Card(source_card),
                );
            }
            sa.set_triggering_spell_ability("SourceSA", source_sa.clone());
        }
        if let Some(target_sa) = params.target_sa.as_deref() {
            sa.set_triggering_spell_ability("Target", target_sa.clone());
        } else if let Some(card) = params.target_card.or(params.card) {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Target,
                crate::event::AbilityValue::Card(card),
            );
        } else if let Some(p) = params.target_player {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Target,
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
            "Source: {}, Target: {}",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Source)
                .unwrap_or_default(),
            sa.get_triggering_object_text(crate::ability::AbilityKey::Target)
                .unwrap_or_default()
        )
    }
}
