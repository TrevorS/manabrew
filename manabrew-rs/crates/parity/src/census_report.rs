//! Joins an engine census (`manabrew_engine::census`) with the card scripts.
//!
//! The census says what the engine read and where it fell back; the scripts say
//! how many cards depend on each parameter. Together: parameters that cards
//! carry, that the engine consulted abilities for, and that it never asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

use forge_carddb::CardDatabase;
use serde::{Deserialize, Serialize};

use crate::script_index::{read_card_list, select_cards, trait_lines};

#[derive(Serialize, Deserialize)]
pub struct CensusFile {
    pub params: Vec<CensusParam>,
    pub unhandled: Vec<CensusUnhandled>,
}

#[derive(Serialize, Deserialize)]
pub struct CensusParam {
    pub owner: String,
    pub key: String,
    pub present: u64,
    pub read: u64,
}

#[derive(Serialize, Deserialize)]
pub struct CensusUnhandled {
    pub kind: String,
    pub detail: String,
    pub count: u64,
}

pub fn write_census(path: &Path) -> std::io::Result<()> {
    let report = manabrew_engine::census::report();
    let file = CensusFile {
        params: report
            .params
            .into_iter()
            .map(|(owner, key, usage)| CensusParam {
                owner,
                key,
                present: usage.present,
                read: usage.read,
            })
            .collect(),
        unhandled: report
            .unhandled
            .into_iter()
            .map(|(kind, detail, count)| CensusUnhandled {
                kind: kind.to_string(),
                detail,
                count,
            })
            .collect(),
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(&mut out, &file)?;
    out.flush()
}

const IDENTIFYING_KEYS: &[&str] = &["SP", "AB", "DB", "ST", "Mode", "Event"];

/// Text and AI hints: the rules engine has no reason to read them.
fn is_presentation_key(key: &str) -> bool {
    key.ends_with("Description")
        || key.ends_with("Desc")
        || key.starts_with("AI")
        || matches!(key, "Secondary" | "CostDesc" | "PrecostDesc" | "TgtPrompt")
}

/// Every `"Identifier"` string literal in the engine sources. A script key
/// that is not among them cannot be handled anywhere, whatever the run played.
fn engine_literals(dir: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for literal in text.split('"').skip(1).step_by(2) {
                    if !literal.is_empty()
                        && literal
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        out.insert(literal.to_string());
                    }
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(dir, &mut out);
    out
}

fn known_subtypes(type_lists: &str) -> BTreeSet<String> {
    type_lists
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('[') && !line.starts_with('#'))
        .map(|line| line.split(':').next().unwrap_or(line).trim().to_lowercase())
        .collect()
}

/// A core type or supertype: `CardTypeLine::has_string_type` answers these, so
/// reaching the subtype tail with one is not a fallback.
fn is_type_word(word: &str) -> bool {
    let mut chars = word.chars();
    let capitalized: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    forge_foundation::CoreType::from_name(&capitalized).is_some()
        || forge_foundation::Supertype::from_name(&capitalized).is_some()
}

/// `parity census-report <census.json> [--cards-file names.txt] [--top N]`
pub fn run_cli(args: &[String], db: &CardDatabase) -> i32 {
    let Some(census_path) = args.get(1) else {
        eprintln!(
            "usage: parity census-report <census.json> [--cards-file FILE] [--top N] [--engine-src DIR]"
        );
        return 2;
    };
    let mut cards_file = None;
    let mut engine_src = "manabrew-rs/crates/manabrew-engine/src".to_string();
    let mut top = 40usize;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--cards-file" => {
                cards_file = args.get(i + 1).cloned();
                i += 1;
            }
            "--engine-src" => {
                engine_src = args.get(i + 1).cloned().unwrap_or(engine_src);
                i += 1;
            }
            "--top" => {
                top = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(top);
                i += 1;
            }
            other => {
                eprintln!("census-report: unknown argument {other}");
                return 2;
            }
        }
        i += 1;
    }

    let census: CensusFile = match std::fs::read(census_path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
    {
        Ok(census) => census,
        Err(e) => {
            eprintln!("census-report: {census_path}: {e}");
            return 2;
        }
    };

    let names = match cards_file.as_deref().map(Path::new).map(read_card_list) {
        Some(Ok(names)) => Some(names),
        Some(Err(e)) => {
            eprintln!("census-report: {e}");
            return 2;
        }
        None => None,
    };
    let (cards, missing) = select_cards(db, names.as_deref());

    let mut cards_with: BTreeMap<(String, String), BTreeSet<&str>> = BTreeMap::new();
    for (name, rules) in &cards {
        for line in trait_lines(rules) {
            let owner = line.owner();
            for (key, _) in &line.params {
                cards_with
                    .entry((owner.clone(), key.to_string()))
                    .or_default()
                    .insert(name.as_str());
            }
        }
    }

    let mut reads_by_owner: BTreeMap<&str, u64> = BTreeMap::new();
    let mut usage: BTreeMap<(&str, &str), (u64, u64)> = BTreeMap::new();
    for p in &census.params {
        *reads_by_owner.entry(p.owner.as_str()).or_default() += p.read;
        usage.insert((p.owner.as_str(), p.key.as_str()), (p.present, p.read));
    }

    println!(
        "scripts: {} cards ({} named cards not in the database); census: {} owners exercised",
        cards.len(),
        missing.len(),
        reads_by_owner.len()
    );

    let literals = engine_literals(Path::new(&engine_src));
    if !literals.contains("ValidTgts") {
        eprintln!(
            "census-report: {engine_src} does not look like the engine sources \
             (no \"ValidTgts\" literal); run from the repo root or pass --engine-src"
        );
        return 2;
    }

    let mut absent: Vec<(usize, &str, &str)> = Vec::new();
    let mut never_read: Vec<(usize, &str, &str, u64)> = Vec::new();
    let mut not_exercised: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for ((owner, key), names) in &cards_with {
        if owner == "?" || IDENTIFYING_KEYS.contains(&key.as_str()) || is_presentation_key(key) {
            continue;
        }
        if !literals.contains(key.as_str()) {
            absent.push((names.len(), owner.as_str(), key.as_str()));
            continue;
        }
        if reads_by_owner.get(owner.as_str()).copied().unwrap_or(0) == 0 {
            not_exercised
                .entry(owner.as_str())
                .or_default()
                .extend(names.iter().copied());
            continue;
        }
        let (present, read) = usage
            .get(&(owner.as_str(), key.as_str()))
            .copied()
            .unwrap_or((0, 0));
        if read == 0 && present > 0 {
            never_read.push((names.len(), owner.as_str(), key.as_str(), present));
        }
    }
    absent.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| (a.1, a.2).cmp(&(b.1, b.2))));
    never_read.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| (a.1, a.2).cmp(&(b.1, b.2))));

    println!(
        "\nparameters with no matching string anywhere in the engine sources ({}):",
        absent.len()
    );
    println!("  {:>5}  owner.key", "cards");
    for (count, owner, key) in absent.iter().take(top) {
        println!("  {count:>5}  {owner}.{key}");
    }

    println!(
        "\nparameters the engine knows, on abilities this run consulted, never read through \
         a tracked accessor ({}):",
        never_read.len()
    );
    println!("  {:>5}  owner.key", "cards");
    for (count, owner, key, _) in never_read.iter().take(top) {
        println!("  {count:>5}  {owner}.{key}");
    }
    println!(
        "  candidates, not verdicts: an IR builder that reads the raw map directly \
         (`Params::inner`) is not tracked"
    );

    let mut idle: Vec<(usize, &str)> = not_exercised
        .iter()
        .map(|(owner, names)| (names.len(), *owner))
        .collect();
    idle.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    println!("\nowners the run never exercised ({}):", idle.len());
    for (count, owner) in idle.iter().take(top) {
        println!("  {count:>5}  {owner}");
    }

    let subtypes = db
        .archive()
        .map(|archive| known_subtypes(archive.type_lists.as_str()))
        .unwrap_or_default();
    let mut fallbacks: Vec<&CensusUnhandled> = census
        .unhandled
        .iter()
        .filter(|u| {
            !(u.kind.ends_with("-as-subtype")
                && (subtypes.contains(&u.detail.to_lowercase()) || is_type_word(&u.detail)))
        })
        .collect();
    fallbacks.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.detail.cmp(&b.detail)));
    println!(
        "\npermissive fallbacks that fired, real types and subtypes excluded ({}):",
        fallbacks.len()
    );
    for u in fallbacks.iter().take(top) {
        println!("  {:>8}  {:<32} {}", u.count, u.kind, u.detail);
    }
    0
}
