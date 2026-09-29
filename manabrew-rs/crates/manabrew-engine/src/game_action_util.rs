use forge_foundation::ZoneType;

use crate::cost::parse_cost;
use crate::game::GameState;
use crate::ids::PlayerId;
use crate::spellability::{AlternativeCost, SpellAbility};

pub fn get_alternative_costs(
    game: &GameState,
    sa: &SpellAbility,
    activator: PlayerId,
) -> Vec<SpellAbility> {
    let mut alternatives = Vec::new();
    let Some(source_id) = sa.source else {
        return alternatives;
    };
    let source = game.card(source_id);
    if source.zone == ZoneType::Battlefield {
        return alternatives;
    }

    for (index, entry) in crate::staticability::static_ability_alternative_cost::alternative_costs(
        game,
        &game.cards,
        sa,
        source,
        activator,
    )
    .iter()
    .enumerate()
    {
        let mut alternative = sa.clone();
        crate::staticability::static_ability_alternative_cost::apply_alternative_cost_to_sa(
            &mut alternative,
            entry,
        );
        alternative.alt_cost_index = index as u8;
        alternatives.push(alternative);
    }

    let turn = game.turn.turn_number;
    let flashback_costs = source.get_all_flashback_costs();
    let mut flashback_index = 0usize;
    let keywords: Vec<String> = source
        .keywords
        .iter_strings()
        .chain(source.granted_keywords.iter_strings())
        .chain(source.pump_keywords.iter_strings())
        .map(str::to_string)
        .collect();
    for keyword in &keywords {
        if keyword == "Mayhem" || keyword.starts_with("Mayhem:") {
            if source.zone != ZoneType::Graveyard
                || !source.was_discarded()
                || !source.entered_current_zone_this_turn(turn)
            {
                continue;
            }
            if let Some(cost) = source.get_mayhem_cost() {
                alternatives.push(get_graveyard_spell_by_keyword(
                    sa,
                    &cost,
                    AlternativeCost::Mayhem,
                    0,
                ));
            }
            continue;
        }
        if sa.is_land_ability {
            continue;
        }
        if let Some(cost) = keyword.strip_prefix("Escape:") {
            if source.zone != ZoneType::Graveyard {
                continue;
            }
            alternatives.push(get_graveyard_spell_by_keyword(
                sa,
                cost,
                AlternativeCost::Escape,
                0,
            ));
        } else if keyword == "Flashback" || keyword.starts_with("Flashback:") {
            let Some(cost) = flashback_costs.get(flashback_index) else {
                continue;
            };
            let index = flashback_index;
            flashback_index += 1;
            if source.zone != ZoneType::Graveyard {
                continue;
            }
            alternatives.push(get_graveyard_spell_by_keyword(
                sa,
                cost,
                AlternativeCost::Flashback,
                index,
            ));
        } else if let Some(cost) = keyword.strip_prefix("Harmonize:") {
            if source.zone != ZoneType::Graveyard {
                continue;
            }
            alternatives.push(get_graveyard_spell_by_keyword(
                sa,
                cost,
                AlternativeCost::Harmonize,
                0,
            ));
        } else if let Some(cost) = keyword.strip_prefix("Foretell:") {
            if source.zone != ZoneType::Exile
                || !source.foretold
                || source.entered_current_zone_this_turn(turn)
                || activator != source.owner
            {
                continue;
            }
            let mut foretold = sa.clone();
            foretold.alt_cost = Some(AlternativeCost::Foretell);
            foretold.restriction.variables.set_zone(ZoneType::Exile);
            foretold.pay_costs = Some(parse_cost(cost));
            alternatives.push(foretold);
        }
    }

    let plotted = keywords
        .iter()
        .any(|keyword| crate::card::parse_plotted_turn(keyword).is_some());
    if plotted
        && crate::player::can_cast_sorcery(game, activator)
        && source.zone == ZoneType::Exile
        && activator == source.owner
        && !source.entered_current_zone_this_turn(turn)
    {
        let mut plot = sa.clone();
        plot.alt_cost = Some(AlternativeCost::Plot);
        plot.restriction.variables.set_zone(ZoneType::Exile);
        plot.pay_costs = Some(parse_cost("0"));
        alternatives.push(plot);
    }

    alternatives
}

pub fn get_graveyard_spell_by_keyword(
    sa: &SpellAbility,
    cost: &str,
    alt_cost: AlternativeCost,
    alt_cost_index: usize,
) -> SpellAbility {
    let mut new_sa = sa.clone();
    new_sa.pay_costs = Some(parse_cost(cost));
    new_sa.alt_cost = Some(alt_cost);
    new_sa.alt_cost_index = alt_cost_index as u8;
    new_sa.restriction.variables.set_zone(ZoneType::Graveyard);
    new_sa
}
