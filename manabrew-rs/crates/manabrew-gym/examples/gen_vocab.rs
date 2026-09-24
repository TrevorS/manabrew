//! Extends `card_vocab.txt` with every face name of the Standard pool (the sets in the archive's
//! `formats/Sanctioned/Standard.txt`), the survey decks, and the tokens they make.
//! Existing lines keep their position, so ids stay stable; new names are appended sorted.
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use manabrew_gym::data::{read_matchups, repo_root, DECKS_DIR, SURVEY_MATCHUPS};
use manabrew_gym::encode::vocab::VOCAB_FILE;
use parity::runner::load_data;
use parity::utils::decks::resolve_deck_spec;

const STANDARD_FORMAT: &str = "formats/Sanctioned/Standard.txt";

fn main() {
    let data = load_data(None, false).expect("load_data");
    let archive = data.db.archive().expect("cardset archive");
    let standard = archive
        .extras
        .iter()
        .find(|f| f.path.as_str() == STANDARD_FORMAT)
        .expect("Standard format in the archive");
    let sets: BTreeSet<&str> = standard
        .raw
        .as_str()
        .lines()
        .find_map(|l| l.strip_prefix("Sets:"))
        .expect("Sets line")
        .split(',')
        .map(str::trim)
        .collect();

    let mut cards = BTreeSet::new();
    let mut tokens = BTreeSet::new();
    for edition in archive.editions.iter() {
        let raw = edition.raw.as_str();
        let code = raw.lines().find_map(|l| l.strip_prefix("Code="));
        if !code.is_some_and(|c| sets.contains(c.trim())) {
            continue;
        }
        let mut section = "";
        for line in raw.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name;
                continue;
            }
            let body = line.split(" @").next().unwrap_or_default();
            let rest = match body.split_once(' ') {
                Some((number, tail)) if number.chars().any(|c| c.is_ascii_digit()) => tail,
                _ => body,
            };
            match section {
                "metadata" => {}
                "tokens" => {
                    if let Some(script) = rest.split_whitespace().next() {
                        tokens.insert(script.to_string());
                    }
                }
                _ => {
                    if let Some((rarity, name)) = rest.split_once(' ') {
                        if rarity.len() == 1 && rarity.chars().all(|c| c.is_ascii_uppercase()) {
                            cards.insert(name.trim().to_string());
                        }
                    }
                }
            }
        }
    }

    let root = repo_root();
    let decks_dir = root.join(DECKS_DIR);
    for (a, b) in read_matchups(&root.join(SURVEY_MATCHUPS)).expect("survey matchups") {
        for deck in [a, b] {
            let spec = resolve_deck_spec(&deck, &[&decks_dir.to_string_lossy()]).expect("deck");
            cards.extend(spec.into_iter().map(|(name, _)| name));
        }
    }

    let mut names = BTreeSet::new();
    let mut missing = 0;
    for card in &cards {
        let Some(rules) = data.db.get_by_card_name(card) else {
            missing += 1;
            continue;
        };
        names.insert(rules.name());
        for face in std::iter::once(&rules.main_part)
            .chain(rules.other_part.iter())
            .chain(rules.specialized_parts.values())
        {
            names.insert(face.name.clone());
        }
        tokens.extend(rules.tokens.iter().cloned());
    }
    for script in &tokens {
        if let Some(token) = data.token_templates.get(script.as_str()) {
            names.insert(token.card_name.clone());
        }
    }

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(VOCAB_FILE);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let known: HashSet<String> = lines.iter().cloned().collect();
    let before = lines.len();
    lines.extend(
        names
            .into_iter()
            .filter(|n| !n.is_empty() && !known.contains(n)),
    );
    std::fs::write(&path, lines.join("\n") + "\n").expect("write vocab");
    println!(
        "sets={} printings={} tokens={} unresolved={missing} vocab={} (+{})",
        sets.len(),
        cards.len(),
        tokens.len(),
        lines.len(),
        lines.len() - before
    );
}
