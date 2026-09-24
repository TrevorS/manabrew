//! Per-card parity probes and failing-deck shrinking.
//!
//! A probe puts one card into a fixed shell (copies of the card, a few cheap
//! colourless creatures for it to interact with, basic lands of its colour
//! identity) and runs it against a fixed opponent on several seeds. The result
//! attributes a divergence to a card instead of to a twenty-card matchup.
//!
//! `shrink` takes a failing deck pair and removes cards while the matchup still
//! diverges, leaving the cards that are needed to reproduce it.

use std::io::Write;
use std::path::Path;

use forge_carddb::CardDatabase;
use forge_foundation::color::Color;
use rayon::prelude::*;

use crate::comparator::normalize_field;
use crate::deck_generator::{format_inline, DeckSpec};
use crate::protocol::{MatchupResult, Verdict};

pub const DEFAULT_PARTNERS: &str = "Memnite*4|Bronze Sable*4";
pub const DEFAULT_OPPONENT: &str = "Memnite*8|Bronze Sable*8|Plains*24";

pub struct ProbeOptions {
    pub seeds: Vec<u64>,
    pub copies: usize,
    pub lands: usize,
    pub partners: DeckSpec,
    pub opponent: DeckSpec,
}

fn basic_land(color: Color) -> &'static str {
    match color {
        Color::White => "Plains",
        Color::Blue => "Island",
        Color::Black => "Swamp",
        Color::Red => "Mountain",
        Color::Green => "Forest",
    }
}

/// The inline deck spec for one probed card, or why it cannot be built.
pub fn probe_deck(db: &CardDatabase, card: &str, options: &ProbeOptions) -> Result<String, String> {
    let rules = db
        .get_by_card_name(card)
        .ok_or_else(|| "not in the card database".to_string())?;
    let mut deck: DeckSpec = vec![(card.to_string(), options.copies)];
    deck.extend(options.partners.iter().cloned());
    let colors: Vec<Color> = rules.color_identity.iter().collect();
    if colors.is_empty() {
        deck.push(("Plains".to_string(), options.lands));
    } else {
        let share = options.lands / colors.len();
        let mut remainder = options.lands % colors.len();
        for color in colors {
            let extra = usize::from(remainder > 0);
            remainder = remainder.saturating_sub(1);
            deck.push((basic_land(color).to_string(), share + extra));
        }
    }
    Ok(format!("inline:{}", format_inline(&deck)))
}

pub struct ProbeRow {
    pub card: String,
    pub error: Option<String>,
    pub runs: Vec<ProbeRun>,
}

pub struct ProbeRun {
    pub seed: u64,
    pub verdict: Verdict,
    pub used: bool,
    pub uses: usize,
    pub turn: Option<u32>,
    pub field: Option<String>,
    pub subject: Option<String>,
    pub rust: Option<String>,
    pub java: Option<String>,
    pub decision: Option<String>,
}

/// Every name the probed card can be logged under: the name asked for, plus the real name
/// behind a `Variant:UniversesWithin:FlavorName:` alias and each face of a split card.
pub fn coverage_names(db: &CardDatabase, card: &str) -> Vec<String> {
    let mut names = vec![card.to_string()];
    if let Some(rules) = db.get_by_card_name(card) {
        names.push(rules.name());
        names.push(rules.main_part.name.clone());
        if let Some(other) = &rules.other_part {
            names.push(other.name.clone());
        }
    }
    names.sort();
    names.dedup();
    names
}

impl ProbeRun {
    fn from_result(names: &[String], seed: u64, result: &MatchupResult) -> Self {
        let headline = result.first_divergence.as_ref();
        Self {
            seed,
            verdict: result.verdict(),
            used: result
                .covered_cards
                .iter()
                .any(|c| names.iter().any(|name| name == c)),
            uses: names
                .iter()
                .filter_map(|name| result.card_uses.get(name))
                .sum(),
            turn: headline.map(|d| d.turn),
            field: headline.map(|d| normalize_field(&d.field)),
            subject: headline.and_then(|d| d.subject.clone()),
            rust: headline.map(|d| d.rust_value.clone()),
            java: headline.map(|d| d.java_value.clone()),
            decision: result.decision.as_ref().map(|d| {
                format!(
                    "T{} {}: Rust {} / Java {}",
                    d.turn, d.phase, d.rust_value, d.java_value
                )
            }),
        }
    }
}

impl ProbeRow {
    pub fn passed(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| r.verdict == Verdict::Pass)
            .count()
    }

    pub fn used(&self) -> usize {
        self.runs.iter().filter(|r| r.used).count()
    }

    pub fn uses(&self) -> usize {
        self.runs.iter().map(|r| r.uses).sum()
    }

    /// `PASS` only counts as evidence when the card was actually cast or played.
    /// `PASS_STATE` passed every snapshot while the engines asked the agent
    /// different questions somewhere, so the agreement may be luck.
    pub fn summary(&self) -> &'static str {
        if self.error.is_some() {
            "UNBUILDABLE"
        } else if self.runs.iter().any(|r| r.verdict != Verdict::Pass) {
            "DIVERGES"
        } else if self.used() == 0 {
            "NEVER_USED"
        } else if self.runs.iter().any(|r| r.decision.is_some()) {
            "PASS_STATE"
        } else {
            "PASS"
        }
    }

    fn first_decision(&self) -> Option<&str> {
        self.runs.iter().find_map(|r| r.decision.as_deref())
    }

    fn first_failure(&self) -> Option<&ProbeRun> {
        self.runs.iter().find(|r| r.verdict != Verdict::Pass)
    }
}

pub fn probe_cards<F>(
    db: &CardDatabase,
    cards: &[String],
    options: &ProbeOptions,
    run: F,
) -> Vec<ProbeRow>
where
    F: Fn(&str, &str, u64) -> MatchupResult + Sync,
{
    let opponent = format!("inline:{}", format_inline(&options.opponent));
    let done = std::sync::atomic::AtomicUsize::new(0);
    cards
        .par_iter()
        .map(|card| {
            let row = match probe_deck(db, card, options) {
                Err(error) => ProbeRow {
                    card: card.clone(),
                    error: Some(error),
                    runs: vec![],
                },
                Ok(deck) => {
                    let names = coverage_names(db, card);
                    ProbeRow {
                        card: card.clone(),
                        error: None,
                        runs: options
                            .seeds
                            .par_iter()
                            .map(|&seed| {
                                ProbeRun::from_result(&names, seed, &run(&deck, &opponent, seed))
                            })
                            .collect(),
                    }
                }
            };
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            eprintln!(
                "[probe] [{n}/{}] {} {}",
                cards.len(),
                row.summary(),
                row.card
            );
            row
        })
        .collect()
}

fn tsv(value: Option<&str>) -> String {
    value
        .unwrap_or("")
        .chars()
        .map(|c| if c == '\t' || c == '\n' { ' ' } else { c })
        .take(200)
        .collect()
}

pub fn write_tsv(path: &Path, rows: &[ProbeRow]) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(
        out,
        "card\tresult\tpassed\tseeds\tused\tfail_seed\tverdict\tturn\tfield\tsubject\trust\tjava\tdecision\tnote\tuses"
    )?;
    for row in rows {
        let failure = row.first_failure();
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.card,
            row.summary(),
            row.passed(),
            row.runs.len(),
            row.used(),
            failure.map(|f| f.seed.to_string()).unwrap_or_default(),
            failure.map(|f| f.verdict.as_str()).unwrap_or_default(),
            failure
                .and_then(|f| f.turn)
                .map(|t| t.to_string())
                .unwrap_or_default(),
            tsv(failure.and_then(|f| f.field.as_deref())),
            tsv(failure.and_then(|f| f.subject.as_deref())),
            tsv(failure.and_then(|f| f.rust.as_deref())),
            tsv(failure.and_then(|f| f.java.as_deref())),
            tsv(row.first_decision()),
            tsv(row.error.as_deref()),
            row.uses(),
        )?;
    }
    out.flush()
}

pub fn print_summary(rows: &[ProbeRow]) {
    let mut counts = std::collections::BTreeMap::new();
    for row in rows {
        *counts.entry(row.summary()).or_insert(0usize) += 1;
    }
    let mut fields = std::collections::BTreeMap::new();
    for row in rows {
        if let Some(field) = row.first_failure().and_then(|f| f.field.clone()) {
            *fields.entry(field).or_insert(0usize) += 1;
        }
    }
    println!("probed {} cards: {counts:?}", rows.len());
    let mut by_count: Vec<_> = fields.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (field, count) in by_count {
        println!("  {count:4} first diverge on {field}");
    }
}

pub struct ShrinkResult {
    pub deck1: DeckSpec,
    pub deck2: DeckSpec,
    pub runs: usize,
    pub headline: String,
}

fn is_basic(name: &str) -> bool {
    matches!(
        name,
        "Plains" | "Island" | "Swamp" | "Mountain" | "Forest" | "Wastes"
    )
}

fn filler_land(deck: &DeckSpec) -> String {
    deck.iter()
        .filter(|(name, _)| is_basic(name))
        .max_by_key(|(_, count)| *count)
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| "Plains".to_string())
}

/// Replace every entry outside `keep` with the deck's most common basic land,
/// so deck sizes stay the same and nobody decks out early.
fn without(deck: &DeckSpec, keep: &[bool]) -> DeckSpec {
    let filler = filler_land(deck);
    let mut out: DeckSpec = Vec::new();
    let mut fill = 0usize;
    for ((name, count), &kept) in deck.iter().zip(keep) {
        if kept {
            out.push((name.clone(), *count));
        } else {
            fill += count;
        }
    }
    if fill > 0 {
        match out.iter_mut().find(|(name, _)| *name == filler) {
            Some(entry) => entry.1 += fill,
            None => out.push((filler, fill)),
        }
    }
    out
}

/// ddmin over the non-basic entries of both decks. `still_fails` runs the
/// matchup; candidates of one round run in parallel.
pub fn shrink<F>(
    deck1: &DeckSpec,
    deck2: &DeckSpec,
    seed: u64,
    run: F,
) -> Result<ShrinkResult, String>
where
    F: Fn(&str, &str, u64) -> MatchupResult + Sync,
{
    let removable: Vec<(usize, usize)> = [deck1, deck2]
        .iter()
        .enumerate()
        .flat_map(|(d, deck)| {
            deck.iter()
                .enumerate()
                .filter(|(_, (name, _))| !is_basic(name))
                .map(move |(i, _)| (d, i))
        })
        .collect();
    let runs = std::sync::atomic::AtomicUsize::new(0);

    let build = |kept: &[(usize, usize)]| -> (DeckSpec, DeckSpec) {
        let mask = |d: usize, deck: &DeckSpec| -> Vec<bool> {
            deck.iter()
                .enumerate()
                .map(|(i, (name, _))| is_basic(name) || kept.contains(&(d, i)))
                .collect()
        };
        (
            without(deck1, &mask(0, deck1)),
            without(deck2, &mask(1, deck2)),
        )
    };
    let attempt = |kept: &[(usize, usize)]| -> Option<MatchupResult> {
        let (d1, d2) = build(kept);
        runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let result = run(
            &format!("inline:{}", format_inline(&d1)),
            &format!("inline:{}", format_inline(&d2)),
            seed,
        );
        (result.verdict() == Verdict::Fail).then_some(result)
    };

    let mut kept = removable;
    let mut last =
        attempt(&kept).ok_or("the matchup does not diverge when rebuilt as inline decks")?;
    let mut granularity = 2usize;
    while kept.len() >= 2 {
        let chunk = kept.len().div_ceil(granularity);
        let complements: Vec<Vec<(usize, usize)>> = kept
            .chunks(chunk)
            .enumerate()
            .map(|(skip, _)| {
                kept.chunks(chunk)
                    .enumerate()
                    .filter(|(i, _)| *i != skip)
                    .flat_map(|(_, c)| c.iter().copied())
                    .collect()
            })
            .collect();
        let found = complements.par_iter().find_map_first(|candidate| {
            attempt(candidate).map(|result| (candidate.clone(), result))
        });
        match found {
            Some((candidate, result)) => {
                eprintln!("[shrink] {} -> {} cards", kept.len(), candidate.len());
                kept = candidate;
                last = result;
                granularity = granularity.saturating_sub(1).max(2);
            }
            None if granularity >= kept.len() => break,
            None => granularity = (granularity * 2).min(kept.len()),
        }
    }

    let (d1, d2) = build(&kept);
    let headline = last
        .first_divergence
        .as_ref()
        .map(|d| {
            format!(
                "T{} {} {}: Rust={} Java={}",
                d.turn, d.phase, d.field, d.rust_value, d.java_value
            )
        })
        .unwrap_or_default();
    Ok(ShrinkResult {
        deck1: d1,
        deck2: d2,
        runs: runs.into_inner(),
        headline,
    })
}
