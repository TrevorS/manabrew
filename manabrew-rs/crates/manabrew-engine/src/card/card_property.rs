//! Card property matching for targeting and filtering.
//!
//! Mirrors Java's `CardProperty.cardHasProperty()` — evaluates whether a card
//! matches a property string used in `ValidTgts$` filters (e.g. "nonBlack",
//! "OppCtrl", "YouCtrl").

use forge_foundation::color::Color;
use forge_foundation::ZoneType;

use super::valid_filter::{
    check_cmc_condition_with_context, check_counter_condition, check_counters_received_this_turn,
    check_power_condition, check_toughness_condition, is_trigger_remembered, is_triggered_object,
    matches_blocked_this_turn, matches_blocking_valid, matches_card_color, matches_card_state,
    matches_chosen_color_source, matches_context_predicate, matches_controlled_by_reference,
    matches_domain_predicate, matches_entered_this_turn_from, matches_owned_by_valid,
    matches_relation_predicate, matches_valid_card_selector_with_context, matches_was_cast_from,
    raw_attached_to_relation, raw_target_ref, relation_target_player_any, MatchContext,
};
use crate::card::Card;
use crate::parsing::{
    CardColorSelector, CardStateSelector, ContextPredicate, RelationPredicate, TargetRef,
};

pub(crate) fn card_has_property(card: &Card, property: &str, context: MatchContext<'_>) -> bool {
    let source = context.source_card;
    let property = property.trim();
    if property.is_empty() {
        return true;
    }
    if let Some(result) = matches_domain_predicate(property, card, context) {
        return result;
    }
    let (negated, value) = if let Some(stripped) = property.strip_prefix('!') {
        (true, stripped)
    } else {
        (false, property)
    };
    let value_lower = value.to_ascii_lowercase();
    if let Some(key) = crate::parsing::triggered_property_key(value) {
        return is_triggered_object(key, card, context) != negated;
    }
    if negated {
        let positive_match = match value_lower.as_str() {
            "token" => card.is_token,
            "creature" => card.is_creature(),
            "land" => card.is_land(),
            "artifact" => card.type_line.is_artifact(),
            "enchantment" => card.type_line.is_enchantment(),
            "legendary" => card.type_line.is_legendary(),
            "basic" => card.type_line.is_basic(),
            "snow" => card.type_line.is_snow(),
            "outlaw" => card.is_outlaw(),
            _ => card.has_subtype(value),
        };
        return !positive_match;
    }

    match value_lower.as_str() {
        "outlaw" => card.is_outlaw(),
        "self" | "strictlyself" | "card.self" => card.id == source.id,
        "other" | "strictlyother" => card.id != source.id,
        "youctrl" | "youcontrol" | "you" => card.controller == source.controller,
        "youown" => card.owner == source.controller,
        "youdontctrl" => card.controller != source.controller,
        "youdontown" => card.owner != source.controller,
        "isremembered" | "card.isremembered" => source.remembered_cards.contains(&card.id),
        "istriggerremembered" | "card.istriggerremembered" => is_trigger_remembered(card, context),
        "effectsource" | "card.effectsource" => source.effect_source == Some(card.id),
        "oppctrl" | "opponentctrl" | "opponent" => card.controller != source.controller,
        "chosenctrl" => Some(card.controller) == source.chosen_player,
        "oppown" | "opponentown" => card.owner != source.controller,
        "targetedplayerown" | "targetedown" | "targetedowner" => {
            context.targeted_players.contains(&card.owner)
        }
        "iscommander" => card.is_commander,
        "legendary" => card.type_line.is_legendary(),
        "basic" => card.type_line.is_basic(),
        "hasabasiclandtype" => card.has_a_basic_land_type(),
        "adventurecard" => card.is_adventure_card(),
        "snow" => card.type_line.is_snow(),
        "kicked" => card.kicked,
        "teamwork" => card.cast_sa.as_ref().is_some_and(|cast_sa| {
            cast_sa
                .optional_costs
                .contains(&crate::spellability::OptionalCost::Teamwork)
        }),
        "cameundercontrolsincelastupkeep" => card.came_under_control_since_last_upkeep(),
        "noncreature" => !card.is_creature(),
        "nonland" => !card.is_land(),
        "nonlegendary" => !card.type_line.is_legendary(),
        "nonbasic" => !card.type_line.is_basic(),
        "nonsnow" => !card.type_line.is_snow(),
        "token" => card.is_token,
        "nontoken" => !card.is_token,
        "tapped" => card.tapped,
        "untapped" => !card.tapped,
        "startedtheturnuntapped" => !card.started_turn_tapped,
        "startedtheturntapped" => card.started_turn_tapped,
        "multicolor" => card.color.is_multicolor(),
        "colorless" => card.color.is_colorless(),
        "whitesource" => matches_card_color(CardColorSelector::White, card),
        "bluesource" => matches_card_color(CardColorSelector::Blue, card),
        "blacksource" => matches_card_color(CardColorSelector::Black, card),
        "redsource" => matches_card_color(CardColorSelector::Red, card),
        "greensource" => matches_card_color(CardColorSelector::Green, card),
        "colorlesssource" => card.color.is_colorless(),
        "chosencolorsource" => matches_chosen_color_source(card, context),
        "attackingalone" => {
            matches_context_predicate(&ContextPredicate::AttackingAlone, card, context)
        }
        "attacking" => matches_context_predicate(&ContextPredicate::Attacking(None), card, context),
        "attackingyou" => matches_context_predicate(
            &ContextPredicate::Attacking(Some(TargetRef::Source)),
            card,
            context,
        ),
        "blocking" => matches_context_predicate(&ContextPredicate::Blocking(None), card, context),
        "blocked" => matches_context_predicate(&ContextPredicate::Blocked, card, context),
        "unblocked" => matches_context_predicate(&ContextPredicate::Unblocked, card, context),
        "attackedthisturn" => {
            matches_context_predicate(&ContextPredicate::AttackedThisTurn, card, context)
        }
        "attackedthiscombat" => {
            matches_context_predicate(&ContextPredicate::AttackedThisCombat, card, context)
        }
        "blockingsource" => {
            matches_context_predicate(&ContextPredicate::BlockingSource, card, context)
        }
        "blockedbysource" | "blockedbysourcelki" => {
            matches_context_predicate(&ContextPredicate::BlockedBySource, card, context)
        }
        "blockingalone" => {
            matches_context_predicate(&ContextPredicate::BlockingAlone, card, context)
        }
        "blockingcreatureyouctrl" => {
            matches_context_predicate(&ContextPredicate::BlockingCreatureYouCtrl, card, context)
        }
        "isblockedbyremembered" => {
            matches_context_predicate(&ContextPredicate::IsBlockedByRemembered, card, context)
        }
        "samename" => matches_relation_predicate(
            &RelationPredicate::SharesNameWith(TargetRef::Source),
            card,
            context,
        ),
        shares if shares.starts_with("sharesnamewith") => {
            let restriction = value["sharesNameWith".len()..].trim();
            if let Some(valid) = restriction.strip_prefix("Valid ") {
                return matches_relation_predicate(
                    &RelationPredicate::SharesNameWithValid(valid.to_string()),
                    card,
                    context,
                );
            }
            raw_target_ref(restriction).is_some_and(|target| {
                matches_relation_predicate(
                    &RelationPredicate::SharesNameWith(target),
                    card,
                    context,
                )
            })
        }
        shares if shares.starts_with("doesnotsharenamewith") => {
            let restriction = value["doesNotShareNameWith".len()..].trim();
            let relation = match raw_target_ref(restriction) {
                Some(target) => RelationPredicate::DoesNotShareNameWith(target),
                None => RelationPredicate::DoesNotShareNameWithValid(restriction.to_string()),
            };
            matches_relation_predicate(&relation, card, context)
        }
        shares if shares.starts_with("sharescardtypewith") => {
            raw_target_ref(&value["sharesCardTypeWith".len()..]).is_some_and(|target| {
                matches_relation_predicate(
                    &RelationPredicate::SharesCardTypeWith(target),
                    card,
                    context,
                )
            })
        }
        shares if shares.starts_with("sharescolorwith") => {
            raw_target_ref(&value["SharesColorWith".len()..]).is_some_and(|target| {
                matches_relation_predicate(
                    &RelationPredicate::SharesColorWith(target),
                    card,
                    context,
                )
            })
        }
        shares if shares.starts_with("sharescmcwith") => {
            raw_target_ref(&value["SharesCMCWith".len()..]).is_some_and(|target| {
                matches_relation_predicate(
                    &RelationPredicate::SharesManaValueWith(target),
                    card,
                    context,
                )
            })
        }
        shares if shares.starts_with("sharescreaturetypewith") => {
            raw_target_ref(&value["sharesCreatureTypeWith".len()..]).is_some_and(|target| {
                matches_relation_predicate(
                    &RelationPredicate::SharesCreatureTypeWith(target),
                    card,
                    context,
                )
            })
        }
        attacking if attacking.starts_with("attacking ") => {
            raw_target_ref(&value["attacking ".len()..]).is_some_and(|target| {
                matches_context_predicate(&ContextPredicate::Attacking(Some(target)), card, context)
            })
        }
        blocked_by if blocked_by.starts_with("blockedbyvalidthisturn ") => {
            matches_blocked_this_turn(
                &card.blocked_by_this_turn,
                value["blockedByValidThisTurn ".len()..].trim(),
                card,
                context,
            )
        }
        blocked if blocked.starts_with("blockedvalidthisturn ") => matches_blocked_this_turn(
            &card.blocked_this_turn,
            value["blockedValidThisTurn ".len()..].trim(),
            card,
            context,
        ),
        blocking_valid if blocking_valid.starts_with("blockingvalid ") => {
            matches_blocking_valid(value["blockingValid ".len()..].trim(), card, context)
        }
        blocking if blocking.starts_with("blocking ") => {
            raw_target_ref(&value["blocking ".len()..]).is_some_and(|target| {
                matches_context_predicate(&ContextPredicate::Blocking(Some(target)), card, context)
            })
        }
        blocking if blocking.starts_with("blocking") => matches_context_predicate(
            &ContextPredicate::BlockingDefined(value["blocking".len()..].to_string()),
            card,
            context,
        ),
        controlled if controlled.starts_with("controlledby ") => {
            matches_controlled_by_reference(value["ControlledBy ".len()..].trim(), card, context)
        }
        attached if attached.starts_with("attachedto ") => {
            let restriction = value["AttachedTo ".len()..].trim();
            match raw_attached_to_relation(restriction) {
                Some(relation) => matches_relation_predicate(&relation, card, context),
                None => card.attached_to.is_some_and(|host| {
                    matches_valid_card_selector_with_context(
                        &crate::parsing::cached_compiled_selector(restriction),
                        context.game.card(host),
                        context,
                    )
                }),
            }
        }
        owned if owned.starts_with("ownedby ") => {
            let valid = value["OwnedBy ".len()..].trim();
            match raw_target_ref(valid) {
                Some(target) => {
                    matches_relation_predicate(&RelationPredicate::OwnedBy(target), card, context)
                }
                None => matches_owned_by_valid(valid, card, context),
            }
        }
        opponent if opponent.starts_with("opponentof ") => {
            raw_target_ref(&value["OpponentOf ".len()..]).is_some_and(|target| {
                matches_relation_predicate(&RelationPredicate::OpponentOf(target), card, context)
            })
        }
        targeting if targeting.starts_with("istargeting ") => {
            raw_target_ref(&value["IsTargeting ".len()..]).is_some_and(|target| {
                matches_relation_predicate(&RelationPredicate::IsTargeting(target), card, context)
            })
        }
        "inzonebattlefield" => card.zone == forge_foundation::ZoneType::Battlefield,
        "inzonegraveyard" => card.zone == forge_foundation::ZoneType::Graveyard,
        "inzonehand" => card.zone == forge_foundation::ZoneType::Hand,
        "inzoneexile" => card.zone == forge_foundation::ZoneType::Exile,
        "inzonestack" => card.zone == forge_foundation::ZoneType::Stack,
        "damagedby" => card
            .damage_sources_this_turn
            .contains(&context.source_card.id),
        "equippedby" | "enchantedby" | "attachedby" => {
            context.source_card.attached_to == Some(card.id)
        }
        "facedown" => matches_card_state(CardStateSelector::FaceDown, card, context),
        "faceup" => !matches_card_state(CardStateSelector::FaceDown, card, context),
        "paired" => matches_card_state(CardStateSelector::Paired, card, context),
        "pairedwith" => matches_card_state(CardStateSelector::PairedWithSource, card, context),
        "attached" => matches_card_state(CardStateSelector::Attached, card, context),
        "equipped" => matches_card_state(CardStateSelector::Equipped, card, context),
        "enchanted" => matches_card_state(CardStateSelector::Enchanted, card, context),
        "hascounters" => matches_card_state(CardStateSelector::HasCounters, card, context),
        "isimprinted" => matches_card_state(CardStateSelector::IsImprinted, card, context),
        "chosen" => matches_card_state(CardStateSelector::Chosen, card, context),
        "chosencard" | "chosencardstrict" => {
            matches_card_state(CardStateSelector::ChosenCard, card, context)
        }
        "namedcard" => matches_card_state(CardStateSelector::NamedCard, card, context),
        "chosencolor" => matches_card_state(CardStateSelector::ChosenColor, card, context),
        "thisturnentered" => matches_card_state(CardStateSelector::EnteredThisTurn, card, context),
        "thisturnenteredfrom_battlefield" => {
            matches_entered_this_turn_from(ZoneType::Battlefield, card, context)
        }
        entered if entered.starts_with("enteredunder ") => {
            raw_target_ref(&value["EnteredUnder ".len()..]).is_some_and(|target| {
                matches_context_predicate(&ContextPredicate::EnteredUnder(target), card, context)
            })
        }
        "wasdealtdamagethisturn" => {
            matches_card_state(CardStateSelector::WasDealtDamageThisTurn, card, context)
        }
        "dealtdamagethisturn" => {
            matches_card_state(CardStateSelector::DealtDamageThisTurn, card, context)
        }
        "dealtdamagetoany" => {
            matches_card_state(CardStateSelector::DealtDamageToAny, card, context)
        }
        "dealtcombatdamagetoany" => {
            matches_card_state(CardStateSelector::DealtCombatDamageToAny, card, context)
        }
        dealt if dealt.starts_with("dealtcombatdamagethisturn") => {
            let Some(target_text) = value.split_once(' ').map(|(_, target)| target.trim()) else {
                return card
                    .damage_history
                    .damage_done_this_turn
                    .iter()
                    .any(|damage| damage.is_combat && damage.amount > 0);
            };
            let Some(target) = raw_target_ref(target_text) else {
                return false;
            };
            card.damage_history.damage_done_this_turn.iter().any(|damage| {
                damage.is_combat
                    && damage.amount > 0
                    && matches!(
                        damage.target,
                        Some(crate::card::card_damage_history::TrackedEntity::Player(player))
                            if relation_target_player_any(&target, context, |target_player| target_player == player)
                    )
            })
        }
        "historic" => matches_card_state(CardStateSelector::Historic, card, context),
        "modified" => matches_card_state(CardStateSelector::Modified, card, context),
        "issaddled" => matches_card_state(CardStateSelector::Saddled, card, context),
        "issuspected" => card.has_s_var("Suspected"),
        "issolved" => card.is_solved(),
        "harnessed" => card.is_harnessed(),
        "crewedthisturn" => context
            .source_card
            .crewed_by_this_turn
            .contains(&(card.id, card.zone_timestamp)),
        "crewedbysourcethisturn" => card
            .crewed_by_this_turn
            .contains(&(context.source_card.id, context.source_card.zone_timestamp)),
        "sneaked" => {
            card.cast_sa.as_ref().is_some_and(|cast| {
                cast.alt_cost == Some(crate::spellability::AlternativeCost::Sneak)
            }) && !context
                .spell_ability
                .is_some_and(|sa| crate::ability::ability_utils::is_unlinked_from_cast_sa(sa, card))
        }
        mode if mode.starts_with("chosenmode") => {
            card.chosen_mode.as_deref().unwrap_or("") == &value["ChosenMode".len()..]
        }
        not_defined
            if not_defined.starts_with("notdefined") && not_defined != "notdefinedtargeted" =>
        {
            match context.spell_ability {
                Some(sa) => !crate::ability::spell_ability_effect::resolve_defined_cards_for_sa(
                    context.game,
                    sa,
                    &value["NotDefined".len()..],
                )
                .contains(&card.id),
                None => true,
            }
        }
        "mayplaysource" => matches_card_state(CardStateSelector::MayPlaySource, card, context),
        "exiledwithsource" => {
            matches_context_predicate(&ContextPredicate::ExiledWithSource, card, context)
        }
        "toplibrary" => matches_context_predicate(&ContextPredicate::TopLibrary, card, context),
        "suspended" => matches_card_state(CardStateSelector::Suspended, card, context),
        "hasxcost" => matches_card_state(CardStateSelector::HasXCost, card, context),
        "singletarget" => matches_card_state(CardStateSelector::SingleTarget, card, context),
        "promisedgift" => matches_card_state(CardStateSelector::PromisedGift, card, context),
        "isringbearer" => matches_card_state(CardStateSelector::RingBearer, card, context),
        "rememberedplayerctrl" => {
            matches_context_predicate(&ContextPredicate::RememberedPlayerCtrl, card, context)
        }
        "wascast" => card.was_cast(),
        "wascastbyyou" => card.was_cast() && card.controller == context.source_controller,
        was_cast_from if was_cast_from.starts_with("wascastfrom") => {
            matches_was_cast_from(&value[11..], card, context)
        }
        named if named.starts_with("named") => card
            .card_name
            .eq_ignore_ascii_case(&value[5..].trim().replace(';', ",").replace('_', " ")),
        _ if value.starts_with("ManaCost") => {
            let cost = card.mana_cost.short_string();
            match value.strip_prefix("ManaCostPartial") {
                Some(color) => Color::from_name(&color.to_ascii_lowercase())
                    .is_some_and(|color| cost.contains(color.short_name())),
                None => cost == value["ManaCost".len()..],
            }
        }
        _ if value.starts_with("counters_") => check_counter_condition(value, card),
        _ if value.starts_with("countersReceivedThisTurn_") => {
            check_counters_received_this_turn(value, card, context)
        }
        _ => {
            if value_lower.starts_with("cmc") {
                let original_rest = &value[3..];
                check_cmc_condition_with_context(original_rest, card, Some(context))
            } else if value_lower.starts_with("power") {
                check_power_condition(&value["power".len()..], card, context)
            } else if value_lower.starts_with("toughness") {
                check_toughness_condition(&value["toughness".len()..], card, context)
            } else if value_lower.starts_with("totalpt_") {
                crate::parsing::compare::compare_expr(card.power() + card.toughness(), &value[8..])
            } else if let Some(color) = Color::from_name(&value_lower) {
                card.color.has_color(color)
            } else if let Some(keyword) = value.strip_prefix("hasKeyword") {
                card.has_keyword(keyword)
            } else if let Some(keyword_suffix) = value_lower.strip_prefix("with") {
                if keyword_suffix.strip_prefix("out").is_some() {
                    !card.has_keyword(&value[7..])
                } else if !keyword_suffix.is_empty() {
                    card.has_keyword(&value[4..])
                } else {
                    true
                }
            } else if let Some(negated_value) = value_lower.strip_prefix("non") {
                let positive_match = match negated_value {
                    "creature" => card.is_creature(),
                    "land" => card.is_land(),
                    "artifact" => card.type_line.is_artifact(),
                    "enchantment" => card.type_line.is_enchantment(),
                    "token" => card.is_token,
                    "colorless" => card.color.is_colorless(),
                    _ => {
                        if let Some(color) = Color::from_name(negated_value) {
                            card.color.has_color(color)
                        } else {
                            card.has_subtype(&value[3..])
                        }
                    }
                };
                !positive_match
            } else if value_lower == "chosentype" {
                source
                    .chosen_type
                    .as_ref()
                    .is_some_and(|ct| card.has_string_type(ct))
            } else {
                let color_name = value.strip_suffix("Source").unwrap_or(value);
                if let Some(color) = Color::from_name(&color_name.to_lowercase()) {
                    card.color.has_color(color)
                } else if color_name.eq_ignore_ascii_case("Colorless") {
                    card.color.is_colorless()
                } else {
                    crate::census::unhandled("property-as-subtype", value);
                    card.has_string_type(value)
                }
            }
        }
    }
}
