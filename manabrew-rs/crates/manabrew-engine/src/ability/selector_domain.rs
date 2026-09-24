use crate::card::valid_filter::MatchContext;
use crate::card::Card;

pub(crate) fn matches_selector_domain_predicate(
    raw: &str,
    card: &Card,
    context: MatchContext<'_>,
) -> Option<bool> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower == "hasability activated" {
        return Some(!card.activated_abilities.is_empty());
    }
    if lower == "saddledthisturn" {
        let source = context.source_card;
        if source.zone == forge_foundation::ZoneType::Battlefield {
            return Some(source.saddled_by_this_turn().contains(&card.id));
        }
        return Some(
            context
                .game
                .and_then(|game| game.get_lki_snapshot(source.id))
                .is_some_and(|lki| lki.saddled_by.contains(&card.id)),
        );
    }
    if lower.starts_with("castsa ")
        || lower == "canenchantsource"
        || lower.starts_with("hasability ")
    {
        return Some(false);
    }
    if let Some(negated) = match lower.as_str() {
        "cmcchosenevenodd" => Some(false),
        "cmcnotchosenevenodd" => Some(true),
        _ => None,
    } {
        let Some(chosen) = context.source_card.chosen_even_odd.as_deref() else {
            return Some(false);
        };
        let matches_chosen = (card.mana_value() % 2 == 0) == (chosen == "Even");
        return Some(matches_chosen != negated);
    }
    None
}
