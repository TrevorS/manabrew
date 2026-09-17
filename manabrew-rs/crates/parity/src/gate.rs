//! Gate files: one JSON line per matchup, plus a header that records which
//! fields the comparator checked and which build produced the results.
//!
//! `diff` classifies each matchup against a baseline so a gate run says what
//! moved, not only whether the pass count changed.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::comparator::{normalize_field, COMPARED_FIELDS};
use crate::protocol::{MatchupResult, Verdict};

const VALUE_LIMIT: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateHeader {
    pub compared_fields: Vec<String>,
    pub build: String,
    pub max_turns: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateRecord {
    pub deck1: String,
    pub deck2: String,
    pub seed: u64,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub localized: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
}

impl GateRecord {
    pub fn from_result(result: &MatchupResult) -> Self {
        let verdict = result.verdict();
        let headline = result.first_divergence.as_ref();
        let mut fields: Vec<String> = result
            .divergences
            .iter()
            .map(|d| normalize_field(&d.field))
            .collect();
        fields.dedup();
        let detail = match verdict {
            Verdict::Aborted | Verdict::Timeout | Verdict::Skipped => result.skip_reason.clone(),
            Verdict::Error | Verdict::Oom => result.error_message.as_deref().map(clip),
            Verdict::Pass | Verdict::Fail => None,
        };
        Self {
            deck1: result.deck1.clone(),
            deck2: result.deck2.clone(),
            seed: result.seed,
            verdict,
            turn: headline.map(|d| d.turn),
            phase: headline.map(|d| d.phase.clone()),
            field: headline.map(|d| normalize_field(&d.field)),
            subject: headline.and_then(|d| d.subject.clone()),
            rust: headline.map(|d| clip(&d.rust_value)),
            java: headline.map(|d| clip(&d.java_value)),
            fields,
            detail,
            localized: result
                .localized
                .as_ref()
                .map(|d| format!("T{} {} {}", d.turn, d.phase, normalize_field(&d.field))),
            decision: result.decision.as_ref().map(|d| {
                format!(
                    "T{} {}: Rust {} / Java {} ({})",
                    d.turn,
                    d.phase,
                    clip(&d.rust_value),
                    clip(&d.java_value),
                    clip(d.subject.as_deref().unwrap_or("first decision"))
                )
            }),
        }
    }

    fn key(&self) -> (String, String, u64) {
        (self.deck1.clone(), self.deck2.clone(), self.seed)
    }

    fn location(&self) -> String {
        match (&self.turn, &self.field) {
            (Some(turn), Some(field)) => format!("{} T{turn} {field}", self.verdict.as_str()),
            _ => self.verdict.as_str().to_string(),
        }
    }
}

fn clip(value: &str) -> String {
    if value.chars().count() <= VALUE_LIMIT {
        value.to_string()
    } else {
        let clipped: String = value.chars().take(VALUE_LIMIT).collect();
        format!("{clipped}…")
    }
}

pub fn header(build: String, max_turns: u32) -> GateHeader {
    GateHeader {
        compared_fields: COMPARED_FIELDS.iter().map(|f| f.to_string()).collect(),
        build,
        max_turns,
    }
}

pub fn write_file(
    path: &Path,
    header: &GateHeader,
    results: &[MatchupResult],
) -> std::io::Result<()> {
    let mut records: Vec<GateRecord> = results.iter().map(GateRecord::from_result).collect();
    records.sort_by_key(GateRecord::key);
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(out, "{}", serde_json::to_string(header)?)?;
    for record in &records {
        writeln!(out, "{}", serde_json::to_string(record)?)?;
    }
    out.flush()
}

pub fn read_file(path: &Path) -> Result<(GateHeader, Vec<GateRecord>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines = std::io::BufReader::new(file).lines();
    let first = lines
        .next()
        .ok_or_else(|| format!("{}: empty gate file", path.display()))?
        .map_err(|e| e.to_string())?;
    let header: GateHeader =
        serde_json::from_str(&first).map_err(|e| format!("{}: header: {e}", path.display()))?;
    let mut records = Vec::new();
    for (number, line) in lines.enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str(&line)
                .map_err(|e| format!("{}: line {}: {e}", path.display(), number + 2))?,
        );
    }
    Ok((header, records))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Regressed,
    NewlyVisible,
    MovedEarlier,
    Changed,
    MovedLater,
    Fixed,
    Added,
    Removed,
}

impl Change {
    fn label(self) -> &'static str {
        match self {
            Change::Regressed => "REGRESSED",
            Change::NewlyVisible => "NEWLY_VISIBLE",
            Change::MovedEarlier => "MOVED_EARLIER",
            Change::Changed => "CHANGED",
            Change::MovedLater => "MOVED_LATER",
            Change::Fixed => "FIXED",
            Change::Added => "ADDED",
            Change::Removed => "REMOVED",
        }
    }
}

pub struct GateDiff {
    pub same: usize,
    pub changes: Vec<(Change, String)>,
}

fn classify(baseline_fields: &[String], before: &GateRecord, after: &GateRecord) -> Option<Change> {
    // An empty list means the baseline came from a binary that did not record
    // what it compared, so nothing can be called newly compared.
    let newly_compared = !baseline_fields.is_empty()
        && after
            .field
            .as_ref()
            .is_some_and(|field| !baseline_fields.contains(field));
    match (
        before.verdict == Verdict::Pass,
        after.verdict == Verdict::Pass,
    ) {
        (true, true) => None,
        (false, true) => Some(Change::Fixed),
        (true, false) if newly_compared => Some(Change::NewlyVisible),
        (true, false) => Some(Change::Regressed),
        (false, false) => {
            if before.verdict == after.verdict
                && before.turn == after.turn
                && before.field == after.field
            {
                return None;
            }
            match (before.turn, after.turn) {
                (Some(b), Some(a)) if a > b => Some(Change::MovedLater),
                (Some(b), Some(a)) if a < b && newly_compared => Some(Change::NewlyVisible),
                (Some(b), Some(a)) if a < b => Some(Change::MovedEarlier),
                _ => Some(Change::Changed),
            }
        }
    }
}

pub fn diff(
    baseline: &(GateHeader, Vec<GateRecord>),
    observed: &(GateHeader, Vec<GateRecord>),
) -> GateDiff {
    let before: BTreeMap<_, _> = baseline.1.iter().map(|r| (r.key(), r)).collect();
    let after: BTreeMap<_, _> = observed.1.iter().map(|r| (r.key(), r)).collect();
    let mut same = 0usize;
    let mut changes = Vec::new();
    for (key, b) in &before {
        let name = format!("{} {} seed={}", key.0, key.1, key.2);
        match after.get(key) {
            None => changes.push((Change::Removed, format!("{name}: was {}", b.location()))),
            Some(a) => match classify(&baseline.0.compared_fields, b, a) {
                None => same += 1,
                Some(change) => {
                    let subject = a
                        .subject
                        .as_deref()
                        .map(|s| format!(" ({s})"))
                        .unwrap_or_default();
                    changes.push((
                        change,
                        format!("{name}: {} -> {}{subject}", b.location(), a.location()),
                    ));
                }
            },
        }
    }
    for (key, a) in &after {
        if !before.contains_key(key) {
            changes.push((
                Change::Added,
                format!("{} {} seed={}: {}", key.0, key.1, key.2, a.location()),
            ));
        }
    }
    changes.sort();
    GateDiff { same, changes }
}

/// `parity gate-diff <baseline> <observed>`: exit 0 and `GATE_SAME` when every
/// matchup has the same verdict, turn and headline field; otherwise list the
/// changes by class, print `GATE_CHANGED`, and exit 1.
pub fn run_cli(args: &[String]) -> i32 {
    let (Some(baseline_path), Some(observed_path)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: parity gate-diff <baseline.jsonl> <observed.jsonl>");
        return 2;
    };
    let baseline = match read_file(Path::new(baseline_path)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gate-diff: {e}");
            return 2;
        }
    };
    let observed = match read_file(Path::new(observed_path)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gate-diff: {e}");
            return 2;
        }
    };

    let tally = |records: &[GateRecord]| {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for r in records {
            *counts.entry(r.verdict.as_str()).or_default() += 1;
        }
        counts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("baseline: {} [{}]", tally(&baseline.1), baseline.0.build);
    println!("observed: {} [{}]", tally(&observed.1), observed.0.build);
    let strict = |records: &[GateRecord]| {
        records
            .iter()
            .filter(|r| r.verdict == Verdict::Pass && r.decision.is_none())
            .count()
    };
    println!(
        "state and decisions both agree: baseline {}, observed {}",
        strict(&baseline.1),
        strict(&observed.1)
    );

    let added_fields: Vec<&String> = observed
        .0
        .compared_fields
        .iter()
        .filter(|f| !baseline.0.compared_fields.contains(f))
        .collect();
    if !added_fields.is_empty() && !baseline.0.compared_fields.is_empty() {
        println!("fields compared now and not in the baseline: {added_fields:?}");
    }

    let result = diff(&baseline, &observed);
    for (change, line) in &result.changes {
        println!("{:<14} {line}", change.label());
    }
    if result.changes.is_empty() {
        println!("GATE_SAME ({} matchups)", result.same);
        0
    } else {
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for (change, _) in &result.changes {
            *counts.entry(change.label()).or_default() += 1;
        }
        let summary = counts
            .iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("GATE_CHANGED ({} same; {summary})", result.same);
        1
    }
}
