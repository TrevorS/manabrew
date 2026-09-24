//! Card scripts as data: every ability, trigger, static, replacement and SVar
//! line of a card, split into `Key$ Value` pairs.
//!
//! Shared by the census report, the script query and the gate coverage map, so
//! "which cards use X" is answered from parsed scripts and not from a text
//! search whose pattern can silently match nothing.

use forge_carddb::{CardDatabase, CardFace, CardRules};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TraitKind {
    Ability,
    Trigger,
    Static,
    Replacement,
    SVar,
}

impl TraitKind {
    pub fn label(self) -> &'static str {
        match self {
            TraitKind::Ability => "ability",
            TraitKind::Trigger => "trigger",
            TraitKind::Static => "static",
            TraitKind::Replacement => "replacement",
            TraitKind::SVar => "svar",
        }
    }
}

pub struct TraitLine<'a> {
    pub face: &'a str,
    pub kind: TraitKind,
    pub raw: &'a str,
    pub params: Vec<(&'a str, &'a str)>,
}

impl<'a> TraitLine<'a> {
    /// The API, trigger mode or replacement event the line belongs to. Matches
    /// `manabrew_engine::census`, which derives the same name at run time.
    pub fn owner(&self) -> String {
        for api_key in ["SP", "AB", "DB", "ST"] {
            if let Some(api) = self.param(api_key) {
                return api.to_string();
            }
        }
        if let Some(mode) = self.param("Mode") {
            return format!("Mode:{mode}");
        }
        if let Some(event) = self.param("Event") {
            return format!("Event:{event}");
        }
        "?".to_string()
    }

    pub fn param(&self, key: &str) -> Option<&'a str> {
        self.params
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| *value)
    }
}

pub fn parse_params(raw: &str) -> Vec<(&str, &str)> {
    raw.split('|')
        .filter_map(|part| {
            let (key, value) = part.split_once('$')?;
            Some((key.trim(), value.trim()))
        })
        .collect()
}

fn face_lines<'a>(face: &'a CardFace, out: &mut Vec<TraitLine<'a>>) {
    let groups: [(TraitKind, &'a Vec<String>); 4] = [
        (TraitKind::Ability, &face.abilities),
        (TraitKind::Trigger, &face.triggers),
        (TraitKind::Static, &face.static_abilities),
        (TraitKind::Replacement, &face.replacements),
    ];
    for (kind, lines) in groups {
        for raw in lines {
            out.push(TraitLine {
                face: &face.name,
                kind,
                raw,
                params: parse_params(raw),
            });
        }
    }
    for raw in face.svars.values() {
        let params = parse_params(raw);
        if !params.is_empty() {
            out.push(TraitLine {
                face: &face.name,
                kind: TraitKind::SVar,
                raw,
                params,
            });
        }
    }
}

pub fn faces(rules: &CardRules) -> Vec<&CardFace> {
    let mut faces = vec![&rules.main_part];
    faces.extend(rules.other_part.iter());
    let mut specialized: Vec<_> = rules.specialized_parts.iter().collect();
    specialized.sort_by(|a, b| a.0.cmp(b.0));
    faces.extend(specialized.into_iter().map(|(_, face)| face));
    faces
}

pub fn trait_lines(rules: &CardRules) -> Vec<TraitLine<'_>> {
    let mut out = Vec::new();
    for face in faces(rules) {
        face_lines(face, &mut out);
    }
    out
}

/// Keyword names of every face, without their arguments (`Kicker:1 G` -> `Kicker`).
pub fn keyword_names(rules: &CardRules) -> Vec<&str> {
    faces(rules)
        .into_iter()
        .flat_map(|face| face.keywords.iter())
        .map(|keyword| keyword.split(':').next().unwrap_or(keyword).trim())
        .collect()
}

/// Card names from a file with one name per line; `#` starts a comment.
pub fn read_card_list(path: &std::path::Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// The cards to look at: the named ones that exist, or every card in the database.
pub fn select_cards(
    db: &CardDatabase,
    names: Option<&[String]>,
) -> (Vec<(String, &'static CardRules)>, Vec<String>) {
    match names {
        None => {
            let mut all = db.iter();
            all.sort_by(|a, b| a.0.cmp(&b.0));
            (all, vec![])
        }
        Some(names) => {
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for name in names {
                match db.get_by_card_name(name) {
                    Some(rules) => found.push((name.clone(), rules)),
                    None => missing.push(name.clone()),
                }
            }
            (found, missing)
        }
    }
}

/// Decks as card names, for coverage: every card of every named deck file.
pub fn deck_cards(deck: &str, decks_dirs: &[&str]) -> Result<Vec<String>, String> {
    Ok(crate::utils::decks::resolve_deck_spec(deck, decks_dirs)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}
