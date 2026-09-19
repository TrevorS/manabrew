use serde::{Deserialize, Serialize};

use crate::event::RunParams;
use crate::game::GameState;
use crate::parsing::{keys, Params};
use crate::spellability::SpellAbility;
use crate::trigger::TriggerType;

use super::trigger::{Trigger, TriggerBehavior};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerSpellAbilityCastOrCopy {
    pub trigger_type: TriggerType,
    pub valid_card: Option<crate::parsing::CompiledSelector>,
    pub valid_activating_player: Option<crate::parsing::CompiledSelector>,
    #[serde(default)]
    pub valid_sa: Option<String>,
    #[serde(default)]
    pub valid_sa_on_card: Option<String>,
}

impl TriggerSpellAbilityCastOrCopy {
    pub fn parse(mode_str: &str, params: &Params) -> Box<dyn TriggerBehavior> {
        let valid_card = params.selector_cloned(keys::VALID_CARD);
        let valid_activating_player = params.selector_cloned(keys::VALID_ACTIVATING_PLAYER);
        let valid_sa = params.get_cloned(keys::VALID_SA);
        let valid_sa_on_card = params.get_cloned(keys::VALID_SA_ON_CARD);
        let trigger_type = match mode_str {
            "SpellCast" => TriggerType::SpellCast,
            "AbilityCast" => TriggerType::AbilityCast,
            "SpellAbilityCast" => TriggerType::SpellAbilityCast,
            "SpellCastOrCopy" => TriggerType::SpellCastOrCopy,
            "SpellCopied" => TriggerType::SpellCopied,
            "SpellAbilityCopy" => TriggerType::SpellAbilityCopy,
            "SpellCopy" => TriggerType::SpellCopy,
            "SpellCastAll" => TriggerType::SpellCastAll,
            "SpellCastOnce" => TriggerType::SpellCastOnce,
            "SpellCastOfType" => TriggerType::SpellCastOfType,
            _ => panic!("Unsupported spell/ability cast-or-copy mode: {mode_str}"),
        };
        Box::new(Self {
            trigger_type,
            valid_card,
            valid_activating_player,
            valid_sa,
            valid_sa_on_card,
        })
    }
}

#[typetag::serde]
impl TriggerBehavior for TriggerSpellAbilityCastOrCopy {
    fn trigger_type(&self) -> TriggerType {
        self.trigger_type
    }

    fn perform_test(&self, trigger: &Trigger, params: &RunParams, game: &GameState) -> bool {
        let valid_card_matches = match (&self.valid_card, params.spell_card) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(selector), Some(card_id)) => {
                let source = trigger.base.card_trait_base.host_card(game);
                let mut context =
                    crate::card::valid_filter::MatchContext::from_source(source).with_game(game);
                if let Some(sa) = params.source_sa.as_ref() {
                    context = context.with_spell_ability(sa);
                }
                crate::card::valid_filter::matches_valid_card_selector_with_context(
                    selector,
                    game.card(card_id),
                    context,
                )
            }
        };
        let valid_sa_matches = self.valid_sa.as_deref().is_none_or(|filter| {
            params
                .source_sa
                .as_ref()
                .or(params.spell_ability.as_ref())
                .is_some_and(|sa| {
                    crate::spellability::matches_valid_sa(
                        filter,
                        sa,
                        trigger.base.card_trait_base.host_card(game),
                        sa.source.map(|source| game.card(source)),
                    )
                })
        });
        let valid_sa_on_card_matches = self.valid_sa_on_card.as_deref().is_none_or(|filter| {
            let sa = params.source_sa.as_ref().or(params.spell_ability.as_ref());
            match (sa, params.spell_card) {
                (Some(sa), Some(cast)) => matches_valid_sa_on_card(filter, sa, cast, trigger, game),
                _ => false,
            }
        });
        valid_card_matches
            && valid_sa_matches
            && valid_sa_on_card_matches
            && trigger.matches_optional_valid_player_filter(
                &self.valid_activating_player,
                params.activator.or(params.spell_controller),
                game,
            )
    }

    fn set_triggering_objects(
        &self,
        _trigger: &Trigger,
        sa: &mut SpellAbility,
        params: &RunParams,
        _game: &GameState,
    ) {
        // Java: sa.setTriggeringObject(AbilityKey.Card, cause.getHostCard())
        if let Some(card) = params.spell_card {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Card,
                crate::event::AbilityValue::Card(card),
            );
        }
        // TODO: port SpellAbility triggering object (AbilityKey.SpellAbility = cause)
        // TODO: port SpellAbilityTargets triggering object (from cause.getAllTargetChoices)
        if let Some(amount) = params.life_amount {
            sa.set_triggering_object(crate::ability::AbilityKey::LifeAmount, amount.to_string());
        }
        if let Some(lki) = params.card_lki {
            sa.set_triggering_value(
                crate::ability::AbilityKey::CardLKI,
                crate::event::AbilityValue::Card(lki),
            );
        }
        if let Some(p) = params.activator {
            sa.set_triggering_value(
                crate::ability::AbilityKey::Activator,
                crate::event::AbilityValue::Player(p),
            );
        }
        // TODO: port CurrentStormCount triggering object - not yet in RunParams
        // TODO: port CurrentCastSpells triggering object - not yet in RunParams
    }

    fn get_important_stack_objects(&self, _trigger: &Trigger, sa: &SpellAbility) -> String {
        // Java: "Card: {card}, Activator: {activator}, SpellAbility: {sa}"
        // TODO: include SpellAbility in output once SpellAbility triggering object is ported
        format!(
            "Card: {}, Activator: {}, SpellAbility: ",
            sa.get_triggering_object_text(crate::ability::AbilityKey::Card)
                .unwrap_or_default(),
            sa.get_triggering_object_text(crate::ability::AbilityKey::Activator)
                .unwrap_or_default()
        )
    }
}

/// Keep in sync with `SpellAbilityProperty.hasProperty`: a property it does not know falls back to
/// the cast card's own (`sa.getHostCard().hasProperty`).
fn matches_valid_sa_on_card(
    filter: &str,
    sa: &SpellAbility,
    cast: crate::ids::CardId,
    trigger: &Trigger,
    game: &GameState,
) -> bool {
    let cast_card = game.card(cast);
    filter.split(',').map(str::trim).any(|restriction| {
        let (base, properties) = restriction.split_once('.').unwrap_or((restriction, ""));
        if !crate::spellability::matches_valid_sa(base, sa, cast_card, Some(cast_card)) {
            return false;
        }
        properties
            .split('+')
            .map(str::trim)
            .filter(|property| !property.is_empty())
            .all(|property| {
                if let Some(rest) = property.strip_prefix("ManaSpent ") {
                    let (comparator, amount) = rest.split_at(2.min(rest.len()));
                    let host = trigger.base.card_trait_base.host_card(game);
                    let expr = host.get_s_var(amount).unwrap_or(amount).to_string();
                    let cast_sa = SpellAbility::new_simple(Some(cast), cast_card.controller, "");
                    let y = crate::svar::resolve_numeric_value(game, &cast_sa, &expr, 0);
                    let spent = cast_card.paying_mana_to_cast.len() as i32;
                    crate::parsing::compare::compare_expr(spent, &format!("{comparator}{y}"))
                } else if property.eq_ignore_ascii_case("YouCtrl")
                    || property.eq_ignore_ascii_case("OppCtrl")
                {
                    let restriction = format!("{base}.{property}");
                    crate::spellability::matches_valid_sa(
                        &restriction,
                        sa,
                        cast_card,
                        Some(cast_card),
                    )
                } else {
                    let selector =
                        crate::parsing::CompiledSelector::parse(&format!("Card.{property}"));
                    crate::card::valid_filter::matches_valid_card_selector_with_context(
                        &selector,
                        cast_card,
                        crate::card::valid_filter::MatchContext::from_source(cast_card)
                            .with_game(game)
                            .with_spell_ability(sa),
                    )
                }
            })
    })
}
