//! Partial parity module for Java `CardFactoryUtil`.

use crate::HashSet;

use crate::card::Card;
use crate::core::HasSVars;
use crate::parsing::{keys, Params};
use crate::replacement::parse_replacement_effect;
use crate::replacement::ReplacementEffect;
use crate::spellability::SpellAbility;
use crate::staticability::StaticAbility;
use crate::trigger::Trigger;

pub fn ability_cast_face_down(card: &Card, _intrinsic: bool, key: &str) -> SpellAbility {
    SpellAbility::new_simple(Some(card.id), card.controller, &format!("FaceDown:{key}"))
}

pub fn resolve(sa: &SpellAbility, card: &mut Card) {
    if let Some(raw) = sa.ir.add_keywords.as_deref() {
        for kw in raw.split('&').map(str::trim).filter(|s| !s.is_empty()) {
            card.add_intrinsic_keyword(kw);
        }
    }
    if let Some(raw) = sa.ir.add_types.as_deref() {
        for ty in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            card.add_type(ty);
        }
    }
}

pub fn can_play(sa: &SpellAbility, card: &Card) -> bool {
    sa.source == Some(card.id) && sa.activating_player == card.controller
}

pub fn ability_unlock_room(card: &Card) -> SpellAbility {
    SpellAbility::new_simple(Some(card.id), card.controller, "UnlockRoom")
}

pub fn face_up_keyword_cost(card: &Card, keyword: &str) -> Option<String> {
    match card.face_down_state.as_deref() {
        Some(state) => crate::keyword::extract_keyword_cost_from_all(
            [&state.original_keywords, &card.granted_keywords],
            keyword,
        ),
        None => card.get_keyword_cost(keyword),
    }
}

pub fn ability_morph_up(card: &mut Card, morph_details: &str, mega: bool, disguise: bool) {
    let mut details = morph_details.split(':');
    let morph_cost = details
        .next()
        .and_then(|cost| cost.split('|').next())
        .unwrap_or_default()
        .trim();
    let reduce_param = details
        .next()
        .filter(|_| disguise)
        .map(|reduce| format!(" | ReduceCost$ {reduce}"))
        .unwrap_or_default();
    let mega_param = if mega { " | Mega$ True" } else { "" };
    let up_key = if disguise { "DisguiseUp" } else { "MorphUp" };
    let text = format!(
        "AB$ SetState | Cost$ {morph_cost} | Mode$ TurnFaceUp | {up_key}$ True{mega_param}{reduce_param}"
    );
    let index = card.activated_abilities.len();
    if let Some(parsed) = crate::ability::activated::parse_activated_ability(&text, index) {
        card.add_intrinsic_activated_ability(parsed);
    }
}

pub fn turn_face_down_with_state(card: &mut Card) {
    card.set_face_down(true);
    card.set_original_state_as_face_down();
    card.set_static_set_pt(None, None);
    let disguise_cost = face_up_keyword_cost(card, "Disguise");
    let megamorph_cost = face_up_keyword_cost(card, "Megamorph");
    if let Some(morph_details) = disguise_cost
        .clone()
        .or_else(|| megamorph_cost.clone())
        .or_else(|| face_up_keyword_cost(card, "Morph"))
    {
        ability_morph_up(
            card,
            &morph_details,
            disguise_cost.is_none() && megamorph_cost.is_some(),
            disguise_cost.is_some(),
        );
    }
}

pub fn set_face_down_state(
    game: &mut crate::game::GameState,
    card_id: crate::ids::CardId,
    sa: &SpellAbility,
) {
    let amount = |key: &str| {
        crate::parsing::raw_get(&sa.ability_text, key)
            .map(|value| crate::svar::resolve_numeric_value(game, sa, value, 0))
    };
    let power = amount("FaceDownPower");
    let toughness = amount("FaceDownToughness");
    let card = game.card_mut(card_id);
    if power.is_some() {
        card.base_power = power;
    }
    if toughness.is_some() {
        card.base_toughness = toughness;
    }
    if let Some(types) = crate::parsing::raw_get(&sa.ability_text, "FaceDownSetType") {
        card.set_type_line(forge_foundation::CardTypeLine::parse(
            &types.split(" & ").collect::<Vec<_>>().join(" "),
        ));
    }
}

pub fn ability_disguise_up(card: &Card, cost_str: &str, _intrinsic: bool) -> SpellAbility {
    SpellAbility::new_simple(
        Some(card.id),
        card.controller,
        &format!("DisguiseUp:{cost_str}"),
    )
}

pub fn ability_turn_face_up(card: &mut Card, key: &str) {
    let Some(state) = card.face_down_state.as_ref() else {
        return;
    };
    if !state.original_type_line.is_creature() || state.original_mana_cost.is_no_cost() {
        return;
    }
    let mut cost = super::alt_costs::mana_cost_script_string(&state.original_mana_cost);
    if cost.is_empty() {
        cost.push('0');
    }
    let text = format!("AB$ SetState | Cost$ {cost} | Mode$ TurnFaceUp | {key}$ True");
    if card
        .activated_abilities
        .iter()
        .any(|ab| ab.ability_text == text)
    {
        return;
    }
    let index = card.activated_abilities.len();
    if let Some(parsed) = crate::ability::activated::parse_activated_ability(&text, index) {
        card.add_intrinsic_activated_ability(parsed);
    }
}

pub fn handle_hidden_agenda(_player: crate::ids::PlayerId, _card: &mut Card) -> bool {
    false
}

pub fn extract_operators(expression: &str) -> String {
    expression
        .chars()
        .filter(|c| matches!(c, '+' | '-' | '*' | '/' | '<' | '>' | '=' | '!'))
        .collect()
}

pub fn sort_colors_from_list(list: &[Card]) -> [i32; 5] {
    let mut out = [0; 5];
    for c in list {
        if c.color.has_white() {
            out[0] += 1;
        }
        if c.color.has_blue() {
            out[1] += 1;
        }
        if c.color.has_black() {
            out[2] += 1;
        }
        if c.color.has_red() {
            out[3] += 1;
        }
        if c.color.has_green() {
            out[4] += 1;
        }
    }
    out
}

pub fn shared_keywords(
    keywords: impl IntoIterator<Item = String>,
    restrictions: &[String],
) -> Vec<String> {
    let restrictions_lc: HashSet<String> = restrictions
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    keywords
        .into_iter()
        .filter(|kw| {
            if restrictions_lc.is_empty() {
                return true;
            }
            restrictions_lc
                .iter()
                .any(|r| kw.to_ascii_lowercase().contains(r))
        })
        .collect()
}

pub fn add_ability_factory_abilities(card: &mut Card, abilities: &[String]) {
    for raw in abilities {
        let sa =
            crate::spellability::build_spell_ability_from_host_card(card, raw, card.controller);
        card.add_spell_ability(&sa);
    }
}

pub fn setup_keyworded_abilities(card: &mut Card) {
    card.generate_keyword_abilities();
    card.generate_keyword_triggers();
    card.base_ability_count = card.activated_abilities.len();
}

/// Generate Dredge replacement effects from the `Dredge:N` keyword.
///
/// Mirrors Java `CardFactoryUtil` Dredge keyword handling which creates a
/// Draw replacement effect:
/// ```text
/// R$ Event$ Draw | ActiveZones$ Graveyard | ValidPlayer$ You
///   | Secondary$ True | Optional$ True
///   | DredgeAmount$ N
///   | Description$ CARDNAME - Dredge N
/// ```
///
/// We use `DredgeAmount$` as a Rust-specific tag (instead of Java's
/// `CheckSVar$` / overriding ability) to keep the implementation simple.
/// The actual mill + return logic is in `replace_draw::execute`.
pub fn add_dredge_replacement(card: &mut Card) {
    let keywords = card.keywords.as_string_list();
    for keyword in keywords {
        let Some(rest) = keyword.strip_prefix("Dredge:") else {
            continue;
        };
        let Ok(amount) = rest.trim().parse::<usize>() else {
            continue;
        };
        let repl_str = format!(
            "R$ Event$ Draw | ActiveZones$ Graveyard | ValidPlayer$ You \
             | Secondary$ True | Optional$ True \
             | DredgeAmount$ {} \
             | Description$ {} - Dredge {}",
            amount, card.card_name, amount
        );
        if let Some(repl) = parse_replacement_effect(&repl_str) {
            card.add_replacement_effect(repl);
        }
    }
}

/// Java parity: convert `ETBReplacement:*` keywords into intrinsic
/// `Event$ Moved` replacement effects during card construction.
///
/// Mirrors `CardFactoryUtil.createETBReplacement(...)` plus the
/// `keyword.startsWith("ETBReplacement")` branch in Java.
pub fn add_etb_keyword_replacements(card: &mut Card) {
    let keywords = card.keywords.as_string_list();
    for keyword in keywords {
        if !keyword.starts_with("ETBReplacement") {
            continue;
        }
        let splitkw: Vec<&str> = keyword.split(':').collect();
        if splitkw.len() < 3 {
            continue;
        }

        let layer = splitkw[1].trim();
        let svar_name = splitkw[2].trim();
        let optional = splitkw.len() >= 4 && splitkw[3].contains("Optional");
        let zone = if splitkw.len() >= 5 {
            splitkw[4].trim()
        } else {
            ""
        };
        let valid = if splitkw.len() >= 6 {
            splitkw[5].trim()
        } else {
            "Card.Self"
        };

        let Some(svar_text) = card.svars.get(svar_name).cloned() else {
            continue;
        };
        let desc = Params::from_raw(&svar_text)
            .get(keys::SPELL_DESCRIPTION)
            .unwrap_or("Replacement effect")
            .replace('|', "/");

        let mut raw = format!(
            "R$ Event$ Moved | Layer$ {layer} | ValidCard$ {valid} | Destination$ Battlefield | ReplacementResult$ Updated | ReplaceWith$ {svar_name} | Description$ {desc}"
        );
        if optional {
            raw.push_str(" | Optional$ True");
        }
        if !zone.is_empty() {
            raw.push_str(" | ActiveZones$ ");
            raw.push_str(zone);
        }

        if let Some(re) = parse_replacement_effect(&raw) {
            card.add_replacement_effect(re);
        }
    }
}

pub fn add_etb_counter_replacements(card: &mut Card) {
    let keywords = card.keywords.as_string_list();
    for keyword in keywords {
        if !keyword
            .split(':')
            .next()
            .is_some_and(|head| head.eq_ignore_ascii_case("etbCounter"))
        {
            continue;
        }
        if let Some(re) = make_etb_counter(&keyword, card, true) {
            card.add_replacement_effect(re);
        }
    }
}

pub fn make_etb_counter(kw: &str, card: &Card, intrinsic: bool) -> Option<ReplacementEffect> {
    let splitkw: Vec<&str> = kw.split(':').collect();
    if splitkw.len() < 3 {
        return None;
    }

    let counter_type = splitkw[1].trim();
    let amount = splitkw[2].trim();
    if counter_type.is_empty() || amount.is_empty() {
        return None;
    }

    let extra_params = splitkw
        .get(3)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && *value != "no Condition");
    let desc = splitkw
        .get(4)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && *value != "no desc")
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "CARDNAME enters with {} {} counter on it.",
                amount,
                counter_type.to_ascii_lowercase()
            )
        });

    let ability_text = format!(
        "DB$ PutCounter | Defined$ Self | CounterType$ {counter_type} | ETB$ True | CounterNum$ {amount}"
    );
    let mut ability = crate::spellability::build_spell_ability_from_host_card(
        card,
        &ability_text,
        card.controller,
    );
    ability.set_intrinsic(intrinsic);

    let mut replacement_text = format!(
        "R$ Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | Secondary$ True | ReplacementResult$ Updated | Description$ {desc}"
    );
    if let Some(extra) = extra_params {
        replacement_text.push_str(" | ");
        replacement_text.push_str(extra);
    }

    let mut replacement = parse_replacement_effect(&replacement_text)?;
    replacement.base.card_trait_base.set_intrinsic(intrinsic);
    replacement.base.set_overriding_ability(ability);
    Some(replacement)
}

pub fn make_read_ahead(card: &Card, intrinsic: bool) -> Option<ReplacementEffect> {
    let ability_text = "DB$ PutCounter | Defined$ Self | CounterType$ LORE | ETB$ True | UpTo$ True | UpToMin$ 1 | CounterNum$ Count$FinalChapterNr";
    let mut ability = crate::spellability::build_spell_ability_from_host_card(
        card,
        ability_text,
        card.controller,
    );
    ability.set_intrinsic(intrinsic);

    let replacement_text = "R$ Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield | Secondary$ True | ReplacementResult$ Updated | Description$ Choose a chapter and start with that many lore counters.";
    let mut replacement = parse_replacement_effect(replacement_text)?;
    replacement.base.card_trait_base.set_intrinsic(intrinsic);
    replacement.base.set_overriding_ability(ability);
    Some(replacement)
}

pub fn add_madness_replacement(card: &mut Card) {
    let keywords = card.keywords.as_string_list();
    for keyword in keywords {
        let Some(cost) = keyword.strip_prefix("Madness:") else {
            continue;
        };
        let cost = cost.trim();
        let desc = if cost == "ManaCost" {
            "Madness: If you discard this card, discard it into exile.".to_string()
        } else {
            let display = forge_foundation::ManaCost::parse(cost);
            format!("Madness {display}: If you discard this card, discard it into exile.")
        };
        let repl_str = format!(
            "R$ Event$ Moved | ActiveZones$ Hand | ValidCard$ Card.Self | Discard$ True \
             | Secondary$ True | NewDestination$ Exile \
             | Description$ {desc}"
        );
        if let Some(repl) = parse_replacement_effect(&repl_str) {
            card.add_replacement_effect(repl);
        }
    }
}

pub fn riot_replacement(intrinsic: bool) -> Option<ReplacementEffect> {
    let repl_str =
        "R$ Event$ Moved | Layer$ Other | ValidCard$ Card.Self | Destination$ Battlefield \
         | ReplacementResult$ Updated | Secondary$ True | ReplaceWith$ Riot | Description$ Riot";
    let mut replacement = parse_replacement_effect(repl_str)?;
    replacement.base.card_trait_base.set_intrinsic(intrinsic);
    replacement.base.card_trait_base.set_svar(
        "Riot".to_string(),
        "DB$ Animate | Defined$ Self | Keywords$ Haste | Duration$ Permanent | UnlessCost$ AddCounter<1/P1P1> | UnlessPayer$ You | SpellDescription$ Riot".to_string(),
    );
    Some(replacement)
}

pub const MIRACLE_TRIGGER: &str = "Mode$ Drawn | ValidCard$ Card.Self | Number$ 1 | Secondary$ True | OptionalDecider$ You | Static$ True | TriggerDescription$ CARDNAME - Miracle";

pub fn miracle_svars(details: &str, suffix: &str) -> [(String, String); 3] {
    let mut k = details.split(':');
    let manacost = k.next().unwrap_or_default();
    let mut ab_str_play =
        format!("DB$ Play | Defined$ Self | Optional$ True | PlayCost$ {manacost}");
    if let Some(reduce) = k.next() {
        ab_str_play.push_str(&format!(" | PlayReduceCost$ {reduce}"));
    }
    [
        (
            format!("TrigMiracle{suffix}"),
            format!("DB$ Reveal | Defined$ You | RevealDefined$ Self | SubAbility$ MiracleImmediate{suffix}"),
        ),
        (
            format!("MiracleImmediate{suffix}"),
            format!("DB$ ImmediateTrigger | Execute$ MiraclePlay{suffix} | TriggerDescription$ CARDNAME - Miracle"),
        ),
        (format!("MiraclePlay{suffix}"), ab_str_play),
    ]
}

pub fn add_riot_replacement(card: &mut Card) {
    if !card.keywords.as_string_list().iter().any(|kw| kw == "Riot") {
        return;
    }
    if let Some(repl) = riot_replacement(true) {
        card.add_replacement_effect(repl);
    }
}

pub fn add_daybound_replacement(card: &mut Card) {
    if !card
        .keywords
        .as_string_list()
        .iter()
        .any(|kw| kw == "Daybound")
    {
        return;
    }
    let repl_str = "R$ Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield \
         | DayTime$ Night | Secondary$ True | Layer$ Transform | ReplacementResult$ Updated \
         | ReplaceWith$ DayboundEnterTransformed \
         | Description$ If it is night, this permanent enters transformed.";
    if let Some(mut repl) = parse_replacement_effect(repl_str) {
        repl.base.card_trait_base.set_svar(
            "DayboundEnterTransformed".to_string(),
            "DB$ SetState | Defined$ ReplacedCard | Mode$ Transform | ETB$ True".to_string(),
        );
        card.add_replacement_effect(repl);
    }
}

pub fn add_devour_replacement(card: &mut Card) {
    use crate::keyword::keyword_with_type_interface::KeywordWithTypeTrait;
    let keywords = card.keywords.as_string_list();
    for keyword in keywords {
        let Some(details) = keyword.strip_prefix("Devour:") else {
            continue;
        };
        let mut devour = crate::keyword::devour::Devour::new(keyword.clone());
        devour.parse(details);
        let valid = devour.get_valid_type().to_string();
        card.set_s_var(
            "DevourSac",
            format!(
                "DB$ Sacrifice | Defined$ You | Amount$ DevourSacX | RememberSacrificed$ True | Optional$ True | SacValid$ {valid}.Other | SacMessage$ another {} | SubAbility$ DevourCounter",
                devour.get_type_description()
            ),
        );
        card.set_s_var(
            "DevourCounter",
            "DB$ PutCounter | ETB$ True | Defined$ Self | CounterType$ P1P1 | CounterNum$ DevourX | SubAbility$ DevourCleanup",
        );
        card.set_s_var("DevourCleanup", "DB$ Cleanup | ClearRemembered$ True");
        card.set_s_var("DevourSacX", format!("Count$Valid {valid}.YouCtrl+Other"));
        card.set_s_var(
            "DevourX",
            format!(
                "Count$RememberedSize/Times.{}",
                devour.inner.get_amount_string()
            ),
        );
        let repl_str = format!(
            "R$ Event$ Moved | ValidCard$ Card.Self | Destination$ Battlefield \
             | ReplacementResult$ Updated | ReplaceWith$ DevourSac | Description$ {}",
            devour.get_title()
        );
        if let Some(mut repl) = parse_replacement_effect(&repl_str) {
            repl.base.card_trait_base.set_intrinsic(true);
            card.add_replacement_effect(repl);
        }
    }
}

/// Mirrors Java `CardFactoryUtil.aaFlashback()` — registers a replacement effect
/// that exiles the card instead of sending it to the graveyard from the stack.
/// Java uses `ValidStackSa$ Spell.Flashback+castKeyword` but in practice the
/// replacement fires for ANY card with the Flashback keyword leaving the stack,
/// because `castKeyword` matches the keyword's presence, not the cast mode.
pub fn add_flashback_replacement(card: &mut Card) {
    let keywords = card.keywords.as_string_list();
    if let Some(repl) = keywords
        .iter()
        .find_map(|kw| kw.strip_prefix("Flashback:"))
        .and_then(flashback_replacement)
    {
        card.add_replacement_effect(repl);
    }
}

pub fn flashback_replacement(cost: &str) -> Option<ReplacementEffect> {
    let cost_display = forge_foundation::ManaCost::parse(cost.trim());
    let desc = format!(
        "Flashback {cost_display} (You may cast this card from your graveyard for its flashback cost. Then exile it.)"
    );
    parse_replacement_effect(&format!(
        "R$ Event$ Moved | ValidCard$ Card.Self | Origin$ Stack | ExcludeDestination$ Exile \
         | FlashbackCast$ True | Secondary$ True | NewDestination$ Exile \
         | Description$ {desc}"
    ))
}

pub fn add_harmonize_replacement(card: &mut Card) {
    let keywords = card.keywords.as_string_list();
    let Some(cost) = keywords
        .iter()
        .find_map(|kw| kw.strip_prefix("Harmonize:").map(str::trim))
    else {
        return;
    };
    let cost_display = forge_foundation::ManaCost::parse(cost);
    let desc = format!(
        "Harmonize {cost_display} (You may cast this card from your graveyard for its harmonize cost. You may tap a creature you control to reduce that cost by {{X}}, where X is its power. Then exile this spell.)"
    );
    let repl_str = format!(
        "R$ Event$ Moved | ValidCard$ Card.Self | Origin$ Stack | ExcludeDestination$ Exile \
         | HarmonizeCast$ True | Secondary$ True | NewDestination$ Exile \
         | Description$ {desc}"
    );
    if let Some(repl) = parse_replacement_effect(&repl_str) {
        card.add_replacement_effect(repl);
    }
}

pub fn add_trigger_ability(card: &mut Card, trig: Trigger) {
    card.add_trigger(trig);
}

pub fn add_replacement_effect(card: &mut Card, re: ReplacementEffect) {
    card.add_replacement_effect(re);
}

pub fn add_spell_ability(card: &mut Card, sa: &SpellAbility) {
    card.add_spell_ability(sa);
}

pub fn add_static_ability(card: &mut Card, st: StaticAbility) {
    card.add_static_ability(st);
}

pub fn setup_siege_abilities(card: &mut Card) {
    card.update_triggers();
}

pub fn setup_adventure_ability(card: &mut Card) -> Option<ReplacementEffect> {
    let repeffstr = "R$ Event$ Moved | ValidCard$ Card.Self | Origin$ Stack | ExcludeDestination$ Exile | ValidStackSa$ Spell.Adventure | Fizzle$ False | Secondary$ True | Description$ Adventure";

    let ab_exile = "DB$ ChangeZone | Defined$ Self | Origin$ Stack | Destination$ Exile | StackDescription$ None";
    let mut sa_exile =
        crate::spellability::build_spell_ability_from_host_card(card, ab_exile, card.controller);

    let ab_effect = "DB$ Effect | RememberObjects$ Self | StaticAbilities$ Play | ForgetOnMoved$ Exile | Duration$ Permanent | ConditionDefined$ Self | ConditionPresent$ Card.!copiedSpell+!token | Adventure$ True";
    let sa_effect =
        crate::spellability::build_spell_ability_from_host_card(card, ab_effect, card.controller);

    card.set_s_var(
        "Play",
        "Mode$ Continuous | MayPlay$ True | EffectZone$ Command | Affected$ Card.IsRemembered+!Adventure | AffectedZone$ Exile | Description$ You may cast EFFECTSOURCE.",
    );

    sa_exile.sub_ability = Some(Box::new(sa_effect));

    let mut re = parse_replacement_effect(repeffstr)?;
    re.base.set_overriding_ability(sa_exile);
    Some(re)
}

pub fn setup_omen_ability(card: &Card) -> Option<ReplacementEffect> {
    let repeffstr = "R$ Event$ Moved | ValidCard$ Card.Self | Origin$ Stack | ValidStackSa$ Spell.Omen | Fizzle$ False | Secondary$ True | Description$ Omen";

    let ab_shuffle = "DB$ ChangeZone | Defined$ Self | Origin$ Stack | Destination$ Library | Shuffle$ True | StackDescription$ None";
    let sa_shuffle =
        crate::spellability::build_spell_ability_from_host_card(card, ab_shuffle, card.controller);

    let mut re = parse_replacement_effect(repeffstr)?;
    re.base.set_overriding_ability(sa_shuffle);
    Some(re)
}

pub fn run() {
    let _ = extract_operators("X+Y");
}
