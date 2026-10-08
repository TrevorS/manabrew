use std::collections::BTreeMap;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use parity::deterministic_agent::VerboseMode;
use parity::lockstep::{self, ForgeConfig, ForgeJvm, LockstepEnd, LockstepGame};
use parity::protocol::ParityLogEntry;
use parity::runner::{
    deck_search_dirs, load_data, run_with_data, ActionSpaceManaProbe, LoadedData, RunConfig,
};
use parity::utils::decks::resolve_deck_spec;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const CAUSE_LIMIT: usize = 300;

const LISTED_ENDS: [&str; 6] = [
    "guard_matched",
    "guard_unverified",
    "java_crash",
    "java_runaway_matched",
    "java_runaway",
    "java_timeout",
];

fn listed_summary(records: &BTreeMap<(String, String, u64), Record>) -> String {
    LISTED_ENDS
        .iter()
        .map(|end| {
            let games: Vec<String> = records
                .values()
                .filter(|r| r.end == *end)
                .map(Record::label)
                .collect();
            if games.is_empty() {
                format!("{end} 0")
            } else {
                format!("{end} {} [{}]", games.len(), games.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Record {
    deck1: String,
    deck2: String,
    seed: u64,
    end: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rust_log: Option<String>,
    turn: u32,
}

impl Record {
    fn key(&self) -> (String, String, u64) {
        (self.deck1.clone(), self.deck2.clone(), self.seed)
    }

    fn passed(&self) -> bool {
        (matches!(self.end.as_str(), "game_over" | "turn_cap") || self.listed())
            && self.rust_log.as_deref().is_none_or(|log| log == "SAME")
    }

    fn listed(&self) -> bool {
        LISTED_ENDS.contains(&self.end.as_str())
    }

    fn same_as(&self, other: &Record) -> bool {
        self.end == other.end
            && self.kind == other.kind
            && self.cause == other.cause
            && self.rust_log == other.rust_log
    }

    fn verdict(&self) -> String {
        let mut verdict = self.end.clone();
        if let Some(kind) = &self.kind {
            verdict = format!("{verdict}:{kind}");
        }
        if let Some(log) = self.rust_log.as_deref().filter(|log| *log != "SAME") {
            verdict = format!("{verdict} rust_log={log}");
        }
        verdict
    }

    fn label(&self) -> String {
        format!("{} vs {} seed {}", self.deck1, self.deck2, self.seed)
    }
}

struct Game {
    deck1: String,
    deck2: String,
    seed: u64,
}

struct Run<'a> {
    config: ForgeConfig,
    data: &'a LoadedData,
    dirs: Vec<&'a str>,
    decks_dir: Option<String>,
    max_turns: u32,
    out_dir: PathBuf,
    identity: bool,
}

fn normalized(log: &[ParityLogEntry]) -> Vec<Value> {
    log.iter()
        .map(|entry| {
            let mut value = serde_json::to_value(entry).expect("log entry");
            strip(&mut value);
            value
        })
        .collect()
}

fn strip(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("timestamp_ms");
            for child in map.values_mut() {
                strip(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip),
        _ => {}
    }
}

fn clip(text: &str) -> String {
    let head = text.split(" | state at desync").next().unwrap_or(text);
    let head = head.split(" (then ").next().unwrap_or(head);
    if head.chars().count() <= CAUSE_LIMIT {
        head.to_string()
    } else {
        format!("{}…", head.chars().take(CAUSE_LIMIT).collect::<String>())
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".to_string())
}

fn read_records(path: &Path) -> BTreeMap<(String, String, u64), Record> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Record>(line).expect("record"))
        .map(|record| (record.key(), record))
        .collect()
}

fn diff(baseline: &Path, observed: &Path) -> bool {
    let base = read_records(baseline);
    let seen = read_records(observed);
    let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (key, now) in &seen {
        let Some(was) = base.get(key) else {
            groups
                .entry("new")
                .or_default()
                .push(format!("{}: {}", now.label(), now.verdict()));
            continue;
        };
        if was.same_as(now) {
            continue;
        }
        let group = match (was.passed(), now.passed()) {
            (true, false) => "regressed",
            (false, true) => "fixed",
            _ => "changed",
        };
        let mut line = format!("{}: {} -> {}", now.label(), was.verdict(), now.verdict());
        if let Some(cause) = &now.cause {
            line = format!("{line}\n    now: {cause}");
        }
        if let Some(cause) = &was.cause {
            line = format!("{line}\n    was: {cause}");
        }
        groups.entry(group).or_default().push(line);
    }
    for (key, was) in &base {
        if !seen.contains_key(key) {
            groups.entry("missing").or_default().push(format!(
                "{}: {}",
                was.label(),
                was.verdict()
            ));
        }
    }
    for group in ["regressed", "changed", "fixed", "new", "missing"] {
        if let Some(lines) = groups.get(group) {
            println!("{group} ({}):", lines.len());
            for line in lines {
                println!("  {line}");
            }
        }
    }
    println!(
        "lockstep diff: {} games, {} failing, {}; baseline {} failing, {}",
        seen.len(),
        seen.values().filter(|r| !r.passed()).count(),
        listed_summary(&seen),
        base.values().filter(|r| !r.passed()).count(),
        listed_summary(&base)
    );
    groups.is_empty()
}

fn play(run: &Run, slot: &mut Option<ForgeJvm>, game: &Game) -> (Record, f64) {
    let specs = [
        resolve_deck_spec(&game.deck1, &run.dirs).expect("deck1"),
        resolve_deck_spec(&game.deck2, &run.dirs).expect("deck2"),
    ];
    let stem = format!("{}-{}-{}", game.deck1, game.deck2, game.seed).replace(['/', ':'], "_");
    let java_log = run.out_dir.join(format!("{stem}.java.jsonl"));
    let setup = LockstepGame {
        data: run.data,
        decks: [&specs[0], &specs[1]],
        seed: game.seed,
        max_turns: run.max_turns,
        log: Some(java_log.clone()),
    };
    let mut record = Record {
        deck1: game.deck1.clone(),
        deck2: game.deck2.clone(),
        seed: game.seed,
        end: String::new(),
        kind: None,
        cause: None,
        rust_log: None,
        turn: 0,
    };
    let started = Instant::now();
    let outcome = match catch_unwind(AssertUnwindSafe(|| {
        lockstep::play(slot, &run.config, &setup, &Arc::new(AtomicBool::new(false)))
    })) {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => {
            record.end = "error".to_string();
            record.cause = Some(clip(&e));
            return (record, started.elapsed().as_secs_f64());
        }
        Err(panic) => {
            record.end = "panic".to_string();
            record.cause = Some(clip(&panic_message(panic.as_ref())));
            return (record, started.elapsed().as_secs_f64());
        }
    };
    let secs = started.elapsed().as_secs_f64();
    record.turn = outcome.game.turn.turn_number;
    match &outcome.end {
        LockstepEnd::GameOver => record.end = "game_over".to_string(),
        LockstepEnd::TurnCap => record.end = "turn_cap".to_string(),
        LockstepEnd::GuardMatched(detail) => {
            record.end = "guard_matched".to_string();
            record.cause = Some(clip(detail));
        }
        LockstepEnd::GuardUnverified(detail) => {
            record.end = "guard_unverified".to_string();
            record.cause = Some(clip(detail));
        }
        LockstepEnd::JavaCrash(detail) => {
            record.end = "java_crash".to_string();
            record.cause = Some(clip(detail));
        }
        LockstepEnd::JavaRunawayMatched(detail) => {
            record.end = "java_runaway_matched".to_string();
            record.cause = Some(clip(detail));
        }
        LockstepEnd::JavaRunaway(detail) => {
            record.end = "java_runaway".to_string();
            record.cause = Some(clip(detail));
        }
        LockstepEnd::JavaTimeout(detail) => {
            record.end = "java_timeout".to_string();
            record.cause = Some(clip(detail));
        }
        LockstepEnd::Desync(d) => {
            record.end = "desync".to_string();
            record.kind = Some(d.kind.clone());
            record.cause = Some(clip(&d.detail));
        }
    }
    if run.identity
        && matches!(
            outcome.end,
            LockstepEnd::GameOver | LockstepEnd::TurnCap | LockstepEnd::GuardMatched(_)
        )
    {
        let reference = run_with_data(
            &RunConfig {
                deck1: game.deck1.clone(),
                deck2: game.deck2.clone(),
                seed: game.seed,
                max_turns: run.max_turns,
                cards_dir: None,
                decks_dir: run.decks_dir.clone(),
                verbose: VerboseMode::Off,
                prefer_actions: false,
                deep: false,
                loose_parity: false,
                log_snapshots: false,
                java_heap: "2g".to_string(),
                variant: "Constructed".to_string(),
                commanders: Vec::new(),
                full_log: false,
                live_log: None,
                callback_compare: false,
                localize: false,
                mana_probe: ActionSpaceManaProbe::ComputerUtilMana,
            },
            run.data,
        )
        .expect("parity run");
        let want = normalized(&reference.log);
        let got = normalized(&outcome.rust_log);
        let first_diff = want
            .iter()
            .zip(&got)
            .position(|(a, b)| a != b)
            .or_else(|| (want.len() != got.len()).then(|| want.len().min(got.len())));
        record.rust_log = Some(first_diff.map_or("SAME".to_string(), |i| format!("DIFF@{i}")));
        if let Some(i) = first_diff {
            eprintln!(
                "{}: rust log differs at row {i}\n  want: {}\n  got:  {}",
                record.label(),
                want.get(i).map_or(String::new(), |v| v.to_string()),
                got.get(i).map_or(String::new(), |v| v.to_string())
            );
        }
    }
    if record.passed() {
        let _ = std::fs::remove_file(&java_log);
        let _ = std::fs::remove_file(java_log.with_extension("end.json"));
    } else {
        let lines: Vec<String> = outcome
            .rust_log
            .iter()
            .filter_map(|entry| serde_json::to_string(entry).ok())
            .collect();
        let _ = std::fs::write(
            run.out_dir.join(format!("{stem}.rust.jsonl")),
            lines.join("\n"),
        );
        if let LockstepEnd::Desync(d) = &outcome.end {
            let _ = std::fs::write(run.out_dir.join(format!("{stem}.desync.txt")), &d.detail);
        }
    }
    (record, secs)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "diff" {
        let same = diff(Path::new(&args[2]), Path::new(&args[3]));
        std::process::exit(if same { 0 } else { 1 });
    }
    if args.len() < 6 {
        eprintln!("usage: lockstep <matchups.tsv[,more.tsv]> <seeds> <max_turns> <jar> <out_dir>");
        eprintln!("       lockstep diff <baseline.jsonl> <observed.jsonl>");
        std::process::exit(2);
    }
    let matchups: Vec<(String, String)> = args[1]
        .split(',')
        .flat_map(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("{path}: {e}"))
                .lines()
                .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
                .filter_map(|line| {
                    let mut parts = line.split('\t');
                    Some((parts.next()?.to_string(), parts.next()?.to_string()))
                })
                .filter(|(deck1, _)| deck1 != "deck1")
                .collect::<Vec<_>>()
        })
        .collect();
    let seeds: Vec<u64> = args[2]
        .split(',')
        .map(|s| s.parse().expect("seed"))
        .collect();
    let jobs: usize = std::env::var("LOCKSTEP_JOBS")
        .ok()
        .map_or(1, |n| n.parse().expect("LOCKSTEP_JOBS"));
    let mut games: Vec<Game> = Vec::new();
    for (deck1, deck2) in &matchups {
        for &seed in &seeds {
            games.push(Game {
                deck1: deck1.clone(),
                deck2: deck2.clone(),
                seed,
            });
        }
    }

    let data = load_data(None, false).expect("card data");
    let decks_dir = std::env::var("LOCKSTEP_DECKS_DIR").ok();
    let run = Run {
        config: ForgeConfig {
            jar: PathBuf::from(&args[4]),
            forge_home: std::fs::canonicalize("forge/forge-gui").expect("forge/forge-gui"),
            heap: std::env::var("JAVA_HEAP").unwrap_or_else(|_| "2g".to_string()),
            timeout: Duration::from_secs(300),
        },
        data: &data,
        dirs: deck_search_dirs(decks_dir.as_deref()),
        decks_dir: decks_dir.clone(),
        max_turns: args[3].parse().expect("max turns"),
        out_dir: PathBuf::from(&args[5]),
        identity: std::env::var("LOCKSTEP_IDENTITY").map_or(true, |v| v != "0"),
    };
    std::fs::create_dir_all(&run.out_dir).expect("out dir");

    let next = AtomicUsize::new(0);
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();
    let mut records: Vec<Option<Record>> = vec![None; games.len()];
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, games.len().max(1)) {
            let tx = tx.clone();
            let (run, games, next) = (&run, &games, &next);
            scope.spawn(move || {
                let mut slot = None;
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(game) = games.get(index) else {
                        break;
                    };
                    tx.send((index, play(run, &mut slot, game))).expect("send");
                }
                if let Some(jvm) = slot {
                    jvm.quit();
                }
            });
        }
        drop(tx);
        for (index, (record, secs)) in rx {
            println!(
                "{}\t{}\t{}\t{}\t{}\tturn={}\t{secs:.1}s",
                record.deck1,
                record.deck2,
                record.seed,
                record.verdict(),
                record.cause.as_deref().unwrap_or(""),
                record.turn,
            );
            records[index] = Some(record);
        }
    });

    let records: Vec<Record> = records.into_iter().flatten().collect();
    let mut file =
        std::fs::File::create(run.out_dir.join("lockstep.jsonl")).expect("lockstep.jsonl");
    for record in &records {
        writeln!(file, "{}", serde_json::to_string(record).expect("record")).expect("write");
    }
    let mut failing: BTreeMap<String, usize> = BTreeMap::new();
    for record in records.iter().filter(|r| !r.passed()) {
        *failing.entry(record.verdict()).or_default() += 1;
    }
    let keyed: BTreeMap<(String, String, u64), Record> =
        records.iter().map(|r| (r.key(), r.clone())).collect();
    eprintln!(
        "lockstep: {} games in {:.0}s, failing {failing:?}, {}",
        records.len(),
        started.elapsed().as_secs_f64(),
        listed_summary(&keyed)
    );
    println!(
        "IDENTITY {}/{}",
        records.iter().filter(|r| r.passed()).count(),
        records.len()
    );
}
