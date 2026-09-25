use crate::deck_generator;
use std::sync::RwLock;

use forge_carddb::{CardDatabase, CardRules};
use forge_foundation::ZoneType;
use manabrew_engine::card::CardInstance;
use manabrew_engine::game::GameState;
use manabrew_engine::ids::PlayerId;
use manabrew_engine::HashMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct DeckCardEntry {
    name: String,
    count: usize,
}

#[derive(Debug, Deserialize)]
struct PresetDeckFile {
    cards: Vec<DeckCardEntry>,
}

fn load_preset_deck(name: &str, decks_dirs: &[&str]) -> Result<Vec<(String, usize)>, String> {
    let mut tried = Vec::with_capacity(decks_dirs.len());
    for dir in decks_dirs {
        let path = std::path::Path::new(dir).join(format!("{name}.json"));
        if !path.exists() {
            tried.push(path.display().to_string());
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read preset deck '{}': {}", path.display(), e))?;
        let deck: PresetDeckFile = serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse '{}': {}", path.display(), e))?;
        return Ok(deck.cards.into_iter().map(|c| (c.name, c.count)).collect());
    }
    Err(format!(
        "Preset deck '{}' not found. Searched: {}",
        name,
        tried.join(", ")
    ))
}

pub fn available_presets(decks_dirs: &[&str]) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for dir in decks_dirs {
        let path = std::path::Path::new(dir);
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        names.insert(stem.to_string());
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}
pub fn resolve_deck_spec(spec: &str, decks_dirs: &[&str]) -> Result<Vec<(String, usize)>, String> {
    if let Some(inline) = spec.strip_prefix("inline:") {
        deck_generator::parse_inline(inline)
    } else if let Some(path) = spec.strip_prefix("file:") {
        parse_deck_file(path)
    } else {
        load_preset_deck(spec, decks_dirs)
    }
}

fn parse_deck_file(path: &str) -> Result<Vec<(String, usize)>, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read '{path}': {e}"))?;
    let mut deck = Vec::new();
    for (line_num, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on first whitespace: "4 Lightning Bolt" -> ("4", "Lightning Bolt")
        let (count_str, name) = line.split_once(char::is_whitespace).ok_or_else(|| {
            format!(
                "Line {}: expected 'Count CardName', got '{}'",
                line_num + 1,
                line
            )
        })?;
        let count: usize = count_str.trim().parse().map_err(|_| {
            format!(
                "Line {}: invalid count '{}' in '{}'",
                line_num + 1,
                count_str,
                line
            )
        })?;
        let name = name.trim();
        if name.is_empty() {
            return Err(format!(
                "Line {}: empty card name in '{}'",
                line_num + 1,
                line
            ));
        }
        deck.push((name.to_string(), count));
    }
    if deck.is_empty() {
        return Err(format!("Deck file '{path}' contains no cards"));
    }
    Ok(deck)
}
pub fn build_deck_from_spec(
    game: &mut GameState,
    db: &CardDatabase,
    owner: PlayerId,
    spec: &[(String, usize)],
    verbose: bool,
) {
    build_deck(game, db, owner, spec, verbose, |_, rules, edition| {
        let mut card = CardInstance::from_rules(rules, owner);
        card.set_code = edition.clone();
        card
    });
}

/// Keep in sync with `Card::from_rules`: it reads only the rules and the owner and draws no
/// global ids, so a clone of the card it built for one game equals a fresh build for the next.
#[derive(Default)]
pub struct CardTemplates(RwLock<HashMap<(String, PlayerId), CardInstance>>);

impl CardTemplates {
    fn card(
        &self,
        name: &str,
        rules: &CardRules,
        edition: &Option<String>,
        owner: PlayerId,
    ) -> CardInstance {
        let key = (name.to_string(), owner);
        if let Some(card) = self.0.read().expect("card templates lock").get(&key) {
            return card.clone();
        }
        let mut card = CardInstance::from_rules(rules, owner);
        card.set_code = edition.clone();
        self.0
            .write()
            .expect("card templates lock")
            .entry(key)
            .or_insert(card)
            .clone()
    }
}

pub fn build_deck_from_templates(
    game: &mut GameState,
    templates: &CardTemplates,
    db: &CardDatabase,
    owner: PlayerId,
    spec: &[(String, usize)],
    verbose: bool,
) {
    build_deck(game, db, owner, spec, verbose, |name, rules, edition| {
        templates.card(name, rules, edition, owner)
    });
}

fn build_deck(
    game: &mut GameState,
    db: &CardDatabase,
    owner: PlayerId,
    spec: &[(String, usize)],
    verbose: bool,
    mut make_card: impl FnMut(&str, &CardRules, &Option<String>) -> CardInstance,
) {
    for (name, count) in spec {
        match db
            .get_by_card_name(name)
            .filter(|rules| !rules.is_variant())
        {
            Some(rules) => {
                let edition = db.card_default_edition(name).map(|s| s.to_string());
                for _ in 0..*count {
                    let id = game.create_card(make_card(name, rules, &edition));
                    game.move_card(id, ZoneType::Library, owner);
                }
            }
            None => {
                if verbose {
                    eprintln!("[parity] Unknown card '{name}' — skipped");
                }
            }
        }
    }
}
