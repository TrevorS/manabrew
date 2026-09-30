use crate::CardFace;

pub fn valid(input: &CardFace, valid: &str) -> bool {
    let (k0, rest) = match valid.split_once('.') {
        Some((k0, rest)) => (k0, Some(rest)),
        None => (valid, None),
    };

    let type_matches = match k0 {
        "Card" => true,
        "Permanent" => !input.type_line.is_instant() && !input.type_line.is_sorcery(),
        _ => input.type_line.has_string_type(k0),
    };
    if !type_matches {
        return false;
    }
    if let Some(rest) = rest {
        for m in rest.split('+') {
            if m.contains("ManaCost") {
                if !has_mana_cost(input, m.get(8..).unwrap_or_default()) {
                    return false;
                }
            } else if m.contains("cmcEQ") {
                if !m
                    .get(5..)
                    .and_then(|value| value.parse::<i32>().ok())
                    .is_some_and(|value| has_cmc(input, value))
                {
                    return false;
                }
            } else if !has_property(input, m) {
                return false;
            }
        }
    }

    true
}

fn has_property(input: &CardFace, v: &str) -> bool {
    match v.strip_prefix("non") {
        Some(rest) => !has_property(input, rest),
        None => input.type_line.has_string_type(v),
    }
}

fn has_mana_cost(input: &CardFace, mana_cost: &str) -> bool {
    mana_cost == input.mana_cost.short_string()
}

fn has_cmc(input: &CardFace, value: i32) -> bool {
    input.mana_cost.cmc() == value
}
