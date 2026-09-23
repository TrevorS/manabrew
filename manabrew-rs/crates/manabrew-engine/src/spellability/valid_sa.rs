use crate::ability::api_type::ApiType;
use crate::card::valid_filter::MatchContext;
use crate::card::Card;
use crate::keyword::keyword_instance::Keyword;
use crate::spellability::SpellAbility;

pub fn matches_valid_sa(
    filter: &str,
    sa: &SpellAbility,
    source: &Card,
    ability_host: Option<&Card>,
) -> bool {
    matches_valid_sa_with_context(filter, sa, source, ability_host, None)
}

pub fn matches_valid_sa_with_context(
    filter: &str,
    sa: &SpellAbility,
    source: &Card,
    ability_host: Option<&Card>,
    context: Option<MatchContext<'_>>,
) -> bool {
    let filter = filter.trim();
    if filter.is_empty() {
        return true;
    }

    filter
        .split(',')
        .map(str::trim)
        .filter(|restriction| !restriction.is_empty())
        .any(|restriction| matches_restriction(restriction, sa, source, ability_host, context))
}

fn matches_restriction(
    restriction: &str,
    sa: &SpellAbility,
    source: &Card,
    ability_host: Option<&Card>,
    context: Option<MatchContext<'_>>,
) -> bool {
    let mut parts = restriction.splitn(2, '.');
    let base = parts.next().unwrap_or("").trim();
    let properties = parts.next();

    let base_matches = matches_base_token(base, sa, ability_host);
    if !base_matches {
        return false;
    }

    if let Some(properties) = properties {
        for property in properties
            .split('+')
            .map(str::trim)
            .filter(|property| !property.is_empty())
        {
            if !matches_property_token(property, sa, source, ability_host, context) {
                return false;
            }
        }
    }

    true
}

fn matches_base_token(token: &str, sa: &SpellAbility, ability_host: Option<&Card>) -> bool {
    let token = token.trim();
    let (negated, token) = match token.strip_prefix('!') {
        Some(stripped) => (true, stripped),
        None => (false, token),
    };

    let matched = match token.to_ascii_lowercase().as_str() {
        "spell" => sa.is_spell,
        "ability" => !sa.is_spell,
        "activated" => sa.is_activated,
        "trigger" | "triggered" => sa.is_trigger,
        "spellability" => true,
        "instant" => ability_host.is_some_and(|card| card.type_line.is_instant()),
        "sorcery" => ability_host.is_some_and(|card| card.type_line.is_sorcery()),
        _ => false,
    };

    matched != negated
}

fn matches_property_token(
    token: &str,
    sa: &SpellAbility,
    source: &Card,
    ability_host: Option<&Card>,
    context: Option<MatchContext<'_>>,
) -> bool {
    let token = token.trim();
    let (negated, token) = match token.strip_prefix('!') {
        Some(stripped) => (true, stripped),
        None => (false, token),
    };

    let matched = matches_property_token_positive(token, sa, source, ability_host, context);
    matched != negated
}

fn matches_property_token_positive(
    token: &str,
    sa: &SpellAbility,
    source: &Card,
    ability_host: Option<&Card>,
    context: Option<MatchContext<'_>>,
) -> bool {
    match token.to_ascii_lowercase().as_str() {
        "self" => ability_host.is_some_and(|host| host.id == source.id),
        "youctrl" => sa.activating_player == source.controller,
        "oppctrl" => sa.activating_player != source.controller,
        "manaability" => sa.is_mana_ability || sa.api == Some(ApiType::Mana),
        "nonmanaability" => !(sa.is_mana_ability || sa.api == Some(ApiType::Mana)),
        "istargeting" => sa.target_restrictions.is_some(),
        "isremembered" => sa
            .source
            .is_some_and(|card| source.remembered_cards.contains(&card)),
        "mayplaysource" => sa.may_play_source == Some(source.id),
        "mayhem" => sa.alt_cost == Some(crate::spellability::AlternativeCost::Mayhem),
        "warp" => sa.alt_cost == Some(crate::spellability::AlternativeCost::Warp),
        "sneak" => sa.alt_cost == Some(crate::spellability::AlternativeCost::Sneak),
        "blitz" => sa.alt_cost == Some(crate::spellability::AlternativeCost::Blitz),
        "xcost" => sa.cost_has_x(),
        "singletarget" => sa.targets_single_target(),
        "crew" => is_crew(sa, ability_host),
        "equip" => is_keyword_ability(sa, ability_host, Keyword::Equip, "equip"),
        "saddle" => is_keyword_ability(sa, ability_host, Keyword::Saddle, "saddle"),
        "station" => is_keyword_ability(sa, ability_host, Keyword::Station, "station"),
        "vehicle" | "mount" | "spacecraft" | "planet" => {
            ability_host.is_some_and(|host| host.type_line.has_subtype(token))
        }
        lowered if lowered.starts_with("cmc") && context.is_some() => {
            ability_host.is_some_and(|host| {
                crate::card::valid_filter::check_cmc_condition_with_context(
                    &token[3..],
                    host,
                    context,
                )
            })
        }
        _ => {
            if sa.has_property(token) {
                return true;
            }
            if sa
                .api
                .is_some_and(|api| api.name().eq_ignore_ascii_case(token))
            {
                return true;
            }
            ability_host.is_some_and(|host| {
                crate::card::card_property::card_has_property(host, token, source.controller)
            })
        }
    }
}

fn is_crew(sa: &SpellAbility, ability_host: Option<&Card>) -> bool {
    if ability_text_has_keyword(sa, "crew") {
        return true;
    }

    ability_host.is_some_and(|host| {
        host.has_keyword_enum(Keyword::Crew)
            && sa.api == Some(ApiType::Animate)
            && sa.description.trim_start().starts_with("Crew")
    })
}

fn is_keyword_ability(
    sa: &SpellAbility,
    ability_host: Option<&Card>,
    keyword: Keyword,
    text: &str,
) -> bool {
    if ability_text_has_keyword(sa, text) {
        return true;
    }

    ability_host.is_some_and(|host| {
        host.has_keyword_enum(keyword)
            && sa
                .description
                .trim_start()
                .to_ascii_lowercase()
                .starts_with(text)
    })
}

fn ability_text_has_keyword(sa: &SpellAbility, keyword: &str) -> bool {
    let keyword = keyword.to_ascii_lowercase();
    let description = sa.description.trim_start().to_ascii_lowercase();
    let ability_text = sa.ability_text.to_ascii_lowercase();

    description.starts_with(&keyword)
        || ability_text.contains(&format!("precostdesc$ {keyword}"))
        || ability_text.contains(&format!("spelldescription$ {keyword}"))
}
