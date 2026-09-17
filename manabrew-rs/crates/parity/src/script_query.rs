//! `parity query` and `parity coverage`: questions about card scripts answered
//! from the parsed scripts, each with a control that proves the search works.
//!
//! A text search that matches nothing reads the same as "no card does this".
//! Every query here also reports a count that must be non-zero (how many cards
//! were scanned, how many carry the API at all, whether the control parameter
//! was found), so a zero can be trusted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use forge_carddb::CardDatabase;

use crate::script_index::{
    deck_cards, keyword_names, read_card_list, select_cards, trait_lines, TraitKind,
};

const CONTROL_PARAM: &str = "ValidTgts";

#[derive(Default)]
struct Options {
    values: BTreeMap<String, String>,
}

impl Options {
    fn parse(args: &[String], allowed: &[&str]) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        let mut i = 1;
        while i < args.len() {
            let flag = args[i]
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument {}", args[i]))?;
            if !allowed.contains(&flag) {
                return Err(format!(
                    "unknown option --{flag} (known: {})",
                    allowed.join(", ")
                ));
            }
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("--{flag} needs a value"))?;
            values.insert(flag.to_string(), value.clone());
            i += 2;
        }
        Ok(Self { values })
    }

    fn get(&self, flag: &str) -> Option<&str> {
        self.values.get(flag).map(String::as_str)
    }
}

fn load_names(options: &Options) -> Result<Option<Vec<String>>, String> {
    options
        .get("cards-file")
        .map(|path| read_card_list(Path::new(path)))
        .transpose()
}

fn engine_files_with_literal(engine_src: &Path, literal: &str) -> Vec<String> {
    fn walk(dir: &Path, needle: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, needle, out);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle))
            {
                out.push(path.display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(engine_src, &format!("\"{literal}\""), &mut out);
    out.sort();
    out
}

/// `parity query [--api A] [--mode M] [--event E] [--param P] [--value V]
///               [--keyword K] [--kind ability|trigger|static|replacement|svar]
///               [--cards-file F] [--show N] [--engine-src DIR]`
pub fn run_query_cli(args: &[String], db: &CardDatabase) -> i32 {
    let options = match Options::parse(
        args,
        &[
            "api",
            "mode",
            "event",
            "param",
            "value",
            "keyword",
            "kind",
            "cards-file",
            "show",
            "engine-src",
        ],
    ) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("query: {e}");
            return 2;
        }
    };
    let names = match load_names(&options) {
        Ok(names) => names,
        Err(e) => {
            eprintln!("query: {e}");
            return 2;
        }
    };
    let show: usize = options
        .get("show")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let kind = match options.get("kind") {
        None => None,
        Some("ability") => Some(TraitKind::Ability),
        Some("trigger") => Some(TraitKind::Trigger),
        Some("static") => Some(TraitKind::Static),
        Some("replacement") => Some(TraitKind::Replacement),
        Some("svar") => Some(TraitKind::SVar),
        Some(other) => {
            eprintln!("query: unknown --kind {other}");
            return 2;
        }
    };
    let owner = options
        .get("api")
        .map(str::to_string)
        .or_else(|| options.get("mode").map(|m| format!("Mode:{m}")))
        .or_else(|| options.get("event").map(|e| format!("Event:{e}")));

    let (cards, missing) = select_cards(db, names.as_deref());
    let mut control_cards = 0usize;
    let mut owner_cards: BTreeSet<&str> = BTreeSet::new();
    let mut hits: BTreeMap<&str, Vec<String>> = BTreeMap::new();

    for (name, rules) in &cards {
        if let Some(keyword) = options.get("keyword") {
            if keyword_names(rules)
                .iter()
                .any(|k| k.eq_ignore_ascii_case(keyword))
            {
                hits.entry(name.as_str())
                    .or_default()
                    .push(format!("K:{keyword}"));
            }
        }
        let mut has_control = false;
        for line in trait_lines(rules) {
            has_control |= line.param(CONTROL_PARAM).is_some();
            if options.get("keyword").is_some() {
                continue;
            }
            if kind.is_some_and(|k| k != line.kind) {
                continue;
            }
            if owner.as_ref().is_some_and(|o| *o != line.owner()) {
                continue;
            }
            owner_cards.insert(name.as_str());
            let matched = match (options.get("param"), options.get("value")) {
                (None, None) => true,
                (Some(param), None) => line.param(param).is_some(),
                (Some(param), Some(value)) => line.param(param).is_some_and(|v| v.contains(value)),
                (None, Some(value)) => line.params.iter().any(|(_, v)| v.contains(value)),
            };
            if matched {
                hits.entry(name.as_str()).or_default().push(format!(
                    "{}: {}",
                    line.kind.label(),
                    line.raw
                ));
            }
        }
        control_cards += usize::from(has_control);
    }

    if control_cards == 0 {
        eprintln!(
            "query: none of the {} scanned cards carries {CONTROL_PARAM}$, so the scripts did not load; \
             check CARDSET_ARCHIVE and --cards-file",
            cards.len()
        );
        return 2;
    }
    println!(
        "scanned {} cards ({} named cards not in the database); control: {control_cards} carry {CONTROL_PARAM}$",
        cards.len(),
        missing.len()
    );
    if options.get("keyword").is_none() {
        println!(
            "cards with a line matching the owner/kind filter alone: {}",
            owner_cards.len()
        );
    }
    println!("cards matching the full query: {}", hits.len());
    for (name, lines) in hits.iter().take(show) {
        println!("  {name}");
        for line in lines.iter().take(2) {
            let clipped: String = line.chars().take(200).collect();
            println!("      {clipped}");
        }
    }
    if hits.len() > show {
        println!("  ... {} more (raise --show)", hits.len() - show);
    }

    let engine_src = options
        .get("engine-src")
        .unwrap_or("manabrew-rs/crates/manabrew-engine/src");
    let engine_src = Path::new(engine_src);
    if engine_files_with_literal(engine_src, CONTROL_PARAM).is_empty() {
        println!(
            "engine: {} is not the engine source tree; skipping the handled-in-Rust check",
            engine_src.display()
        );
        return 0;
    }
    for literal in [
        options.get("param"),
        options.get("api"),
        options.get("mode"),
    ]
    .into_iter()
    .flatten()
    {
        let files = engine_files_with_literal(engine_src, literal);
        if files.is_empty() {
            println!(
                "engine: no \"{literal}\" literal anywhere under {}",
                engine_src.display()
            );
        } else {
            println!(
                "engine: \"{literal}\" appears in {} files, e.g.",
                files.len()
            );
            for file in files.iter().take(6) {
                println!("      {file}");
            }
        }
    }
    0
}

fn features(rules: &forge_carddb::CardRules) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = trait_lines(rules)
        .iter()
        .map(|line| line.owner())
        .filter(|owner| owner != "?")
        .collect();
    out.extend(keyword_names(rules).into_iter().map(|k| format!("K:{k}")));
    out
}

/// `parity coverage (--matchups FILE | --decks a,b,c) [--cards-file POOL] [--decks-dir DIR] [--top N]`
///
/// Which APIs, trigger modes, replacement events and keywords of the pool does
/// the gate's deck list contain at all. A gate cannot observe what none of its
/// decks carries, whatever its result.
pub fn run_coverage_cli(args: &[String], db: &CardDatabase) -> i32 {
    let options = match Options::parse(
        args,
        &["matchups", "decks", "cards-file", "decks-dir", "top"],
    ) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("coverage: {e}");
            return 2;
        }
    };
    let top: usize = options
        .get("top")
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let decks_dirs = crate::runner::deck_search_dirs(options.get("decks-dir"));

    let text = match (options.get("matchups"), options.get("decks")) {
        (Some(matchups), _) => match std::fs::read_to_string(matchups) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("coverage: {matchups}: {e}");
                return 2;
            }
        },
        (None, Some(decks)) => decks.replace(',', "\t"),
        (None, None) => {
            eprintln!(
                "usage: parity coverage (--matchups FILE | --decks a,b,c) [--cards-file POOL] \
                 [--decks-dir DIR] [--top N]"
            );
            return 2;
        }
    };
    let decks: BTreeSet<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .flat_map(|line| line.split('\t'))
        .map(str::trim)
        .collect();
    let mut gate_cards: BTreeSet<String> = BTreeSet::new();
    for deck in &decks {
        match deck_cards(deck, &decks_dirs) {
            Ok(cards) => gate_cards.extend(cards),
            Err(e) => {
                eprintln!("coverage: {e}");
                return 2;
            }
        }
    }

    let names = match load_names(&options) {
        Ok(names) => names,
        Err(e) => {
            eprintln!("coverage: {e}");
            return 2;
        }
    };
    let (pool, _) = select_cards(db, names.as_deref());
    let mut pool_cards_with: BTreeMap<String, usize> = BTreeMap::new();
    let mut covered_pool_cards = 0usize;
    let mut gate_features: BTreeSet<String> = BTreeSet::new();
    for name in &gate_cards {
        if let Some(rules) = db.get_by_card_name(name) {
            gate_features.extend(features(rules));
        }
    }
    for (_, rules) in &pool {
        let card_features = features(rules);
        if card_features.iter().all(|f| gate_features.contains(f)) {
            covered_pool_cards += 1;
        }
        for feature in card_features {
            *pool_cards_with.entry(feature).or_default() += 1;
        }
    }
    if !pool_cards_with.contains_key("K:Flying") {
        eprintln!("coverage: no pool card has Flying, so the scripts did not load");
        return 2;
    }

    let covered = pool_cards_with
        .keys()
        .filter(|f| gate_features.contains(*f))
        .count();
    println!(
        "gate: {} decks, {} distinct cards; pool: {} cards",
        decks.len(),
        gate_cards.len(),
        pool.len()
    );
    println!(
        "pool features (APIs, Mode:, Event:, K:keyword) present in some gate deck: {covered} of {}",
        pool_cards_with.len()
    );
    println!(
        "pool cards whose every feature appears somewhere in the gate: {covered_pool_cards} of {}",
        pool.len()
    );
    let mut uncovered: Vec<(&String, &usize)> = pool_cards_with
        .iter()
        .filter(|(f, _)| !gate_features.contains(*f))
        .collect();
    uncovered.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    println!("\nfeatures no gate deck carries, by pool cards affected:");
    for (feature, count) in uncovered.iter().take(top) {
        println!("  {count:>5}  {feature}");
    }
    0
}
