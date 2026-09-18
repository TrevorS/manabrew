//! `parity explain` and `parity gate-summary`: read a finished run instead of rerunning it.
//!
//! `explain` takes a `--format json` report and, for one game, prints the first RNG draw the
//! two agents disagree on, the first target candidate list that differs, every snapshot field
//! that differs at the divergence turn, the options that differ in the first unequal
//! `$ACTION_SPACE` of the decision's phase, and the callbacks of that phase; with `--turn` or
//! `--around` it prints a window of both logs instead. `gate-summary` tallies one `--gate-out`
//! file without a baseline, and with `--group` clusters its failures.

use std::collections::BTreeMap;

use serde_json::Value;

const JAVA_ONLY_ROWS: [&str; 2] = ["choose_starting_player", "get_ability_to_play"];

fn option<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{head}...")
    }
}

fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn int(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(-1)
}

fn log<'a>(result: &'a Value, side: &str) -> &'a [Value] {
    result
        .get(side)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn is_callback(entry: &Value) -> bool {
    entry.get("entry_type").and_then(Value::as_str) == Some("callback")
}

fn only_in(a: &BTreeMap<String, usize>, b: &BTreeMap<String, usize>) -> Vec<String> {
    a.iter()
        .filter(|(k, n)| b.get(*k).copied().unwrap_or(0) < **n)
        .map(|(k, _)| k.clone())
        .collect()
}

fn is_pass(entry: &Value) -> bool {
    text(entry, "name") == "choose_action" && text(entry, "outcome") == "PassPriority"
}

struct Draw<'a> {
    name: String,
    choices: i64,
    outcome: String,
    entry: &'a Value,
}

impl Draw<'_> {
    /// Both engines print a `choose_action` pick the same way, so the picked card is part
    /// of the key there: the same index into a differently ordered list is a difference.
    fn key(&self) -> (&str, i64, &str, i64, i64, String) {
        let picked = if text(self.entry, "name") == "choose_action" {
            text(self.entry, "outcome")
        } else {
            String::new()
        };
        (
            &self.name,
            self.choices,
            &self.outcome,
            int(self.entry, "turn"),
            int(self.entry, "player"),
            picked,
        )
    }
}

/// Draws that consume RNG, in one vocabulary: Java logs a one-option pick and Rust logs a
/// `pick_many_unique` summary row, and `pick_one` is Rust's `pick_index`.
fn draws(entries: &[Value]) -> Vec<Draw<'_>> {
    let mut out = Vec::new();
    for entry in entries.iter().filter(|e| is_callback(e)) {
        let Some(args) = entry.get("args").and_then(Value::as_array) else {
            continue;
        };
        for arg in args {
            let mut name = text(arg, "name");
            let choices = int(arg, "choices");
            if name.starts_with("pick_many_unique") {
                continue;
            }
            if (name == "pick_one" || name == "pick_index") && choices <= 1 {
                continue;
            }
            if name == "pick_one" {
                name = "pick_index".to_string();
            }
            out.push(Draw {
                name,
                choices,
                outcome: text(arg, "outcome"),
                entry,
            });
        }
    }
    out
}

fn print_draw_diff(result: &Value, context: usize) {
    let rust = draws(log(result, "rust_log"));
    let java = draws(log(result, "java_log"));
    let first = rust
        .iter()
        .zip(java.iter())
        .position(|(r, j)| r.key() != j.key())
        .unwrap_or_else(|| rust.len().min(java.len()));
    println!(
        "draws: rust {} java {}, first difference at #{first}",
        rust.len(),
        java.len()
    );
    if first == rust.len() && first == java.len() {
        return;
    }
    for (tag, list) in [("R", &rust), ("J", &java)] {
        let from = first.saturating_sub(context);
        let to = (first + context + 1).min(list.len());
        for (i, draw) in list.iter().enumerate().take(to).skip(from) {
            println!(
                "  {}{tag} #{i} {}/{}={} | T{} {} P{} {} -> {}",
                if i == first { ">" } else { " " },
                draw.name,
                draw.choices,
                draw.outcome,
                int(draw.entry, "turn"),
                text(draw.entry, "phase"),
                int(draw.entry, "player"),
                text(draw.entry, "name"),
                clip(&text(draw.entry, "outcome"), 140),
            );
        }
    }
}

fn snapshot_at(entries: &[Value], turn: i64) -> Option<&Value> {
    entries.iter().find(|e| {
        e.get("entry_type").and_then(Value::as_str) == Some("snapshot") && int(e, "turn") == turn
    })
}

fn walk(path: &str, rust: &Value, java: &Value, out: &mut Vec<String>) {
    match (rust, java) {
        (Value::Object(r), Value::Object(j)) => {
            let keys: std::collections::BTreeSet<&String> = r.keys().chain(j.keys()).collect();
            for key in keys {
                if key == "timestamp_ms" {
                    continue;
                }
                walk(
                    &format!("{path}.{key}"),
                    r.get(key).unwrap_or(&Value::Null),
                    j.get(key).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        (Value::Array(r), Value::Array(j))
            if r.len() == j.len() && r.iter().chain(j.iter()).all(Value::is_object) =>
        {
            for (i, (a, b)) in r.iter().zip(j.iter()).enumerate() {
                walk(&format!("{path}[{i}]"), a, b, out);
            }
        }
        (Value::Array(r), Value::Array(j))
            if r.len() != j.len() && r.iter().chain(j.iter()).all(|v| v.get("name").is_some()) =>
        {
            let names = |list: &[Value]| -> BTreeMap<String, i64> {
                let mut out = BTreeMap::new();
                for v in list {
                    *out.entry(text(v, "name")).or_default() += 1;
                }
                out
            };
            let (rn, jn) = (names(r), names(j));
            let only = |a: &BTreeMap<String, i64>, b: &BTreeMap<String, i64>| -> Vec<String> {
                a.iter()
                    .filter(|(k, n)| b.get(*k).copied().unwrap_or(0) < **n)
                    .map(|(k, _)| k.clone())
                    .collect()
            };
            out.push(format!(
                "  {path}: {} against {} entries\n      R only: {:?}\n      J only: {:?}",
                r.len(),
                j.len(),
                only(&rn, &jn),
                only(&jn, &rn)
            ));
        }
        _ if rust != java => out.push(format!(
            "  {path}\n      R: {}\n      J: {}",
            clip(&rust.to_string(), 300),
            clip(&java.to_string(), 300)
        )),
        _ => {}
    }
}

fn print_snapshot_diff(result: &Value, turn: i64) {
    let (Some(rust), Some(java)) = (
        snapshot_at(log(result, "rust_log"), turn),
        snapshot_at(log(result, "java_log"), turn),
    ) else {
        println!("snapshot T{turn}: missing on one side");
        return;
    };
    let mut lines = Vec::new();
    walk("", rust, java, &mut lines);
    println!("snapshot T{turn}: {} differing field(s)", lines.len());
    for line in lines {
        println!("{line}");
    }
}

/// `name@id` (plus `#ability_index`) of every option in one `$ACTION_SPACE` row.
fn options(outcome: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for part in outcome.split(" | ") {
        let Some(rest) = part.split("card: ").nth(1) else {
            continue;
        };
        let card = rest.split(',').next().unwrap_or(rest).trim();
        let label = match part.split("ability_index: ").nth(1) {
            Some(index) => format!(
                "{card}#{}",
                index.split([' ', '}']).next().unwrap_or_default()
            ),
            None => card.to_string(),
        };
        *out.entry(label).or_default() += 1;
    }
    out
}

fn print_action_space_diff(result: &Value, turn: i64, phase: &str) {
    let spaces = |side: &str| -> Vec<(i64, BTreeMap<String, usize>)> {
        log(result, side)
            .iter()
            .filter(|e| {
                is_callback(e)
                    && int(e, "turn") == turn
                    && text(e, "phase") == phase
                    && text(e, "name") == "$ACTION_SPACE"
            })
            .map(|e| (int(e, "player"), options(&text(e, "outcome"))))
            .collect()
    };
    let rust = spaces("rust_log");
    let java = spaces("java_log");
    for (i, (r, j)) in rust.iter().zip(java.iter()).enumerate() {
        if r == j {
            continue;
        }
        println!(
            "action space #{i} of T{turn} {phase} (P{} / P{}): rust only {:?}, java only {:?}",
            r.0,
            j.0,
            only_in(&r.1, &j.1),
            only_in(&j.1, &r.1)
        );
        return;
    }
    println!(
        "action spaces of T{turn} {phase}: no difference in the first {} (rust {}, java {})",
        rust.len().min(java.len()),
        rust.len(),
        java.len()
    );
}

/// Java lists target candidates in game order and Rust by name, so each list is a multiset.
fn target_candidates(entries: &[Value]) -> Vec<(&Value, BTreeMap<String, usize>)> {
    entries
        .iter()
        .filter(|e| is_callback(e) && text(e, "name") == "choose_targets_for(candidates)")
        .map(|e| {
            let outcome = text(e, "outcome");
            let mut names = BTreeMap::new();
            for part in outcome
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split("), ")
                .filter(|p| !p.is_empty())
            {
                let name = part.strip_suffix(')').unwrap_or(part);
                *names.entry(format!("{name})")).or_default() += 1;
            }
            (e, names)
        })
        .collect()
}

fn print_candidate_diff(result: &Value) {
    let rust = target_candidates(log(result, "rust_log"));
    let java = target_candidates(log(result, "java_log"));
    for (i, ((entry, r), (_, j))) in rust.iter().zip(java.iter()).enumerate() {
        if r != j {
            println!(
                "target candidates #{i} (T{} {} P{}): rust only {:?}, java only {:?}",
                int(entry, "turn"),
                text(entry, "phase"),
                int(entry, "player"),
                only_in(r, j),
                only_in(j, r)
            );
            return;
        }
    }
    if rust.len() != java.len() {
        println!(
            "target candidates: rust {} list(s), java {}, the first {} agree",
            rust.len(),
            java.len(),
            rust.len().min(java.len())
        );
    }
}

fn print_phase_rows(result: &Value, turn: i64, phase: &str, rows: usize) {
    for (tag, side) in [("R", "rust_log"), ("J", "java_log")] {
        let list: Vec<&Value> = log(result, side)
            .iter()
            .filter(|e| {
                is_callback(e)
                    && int(e, "turn") == turn
                    && text(e, "phase") == phase
                    && !is_pass(e)
                    && text(e, "name") != "$ACTION_SPACE"
                    && text(e, "name") != "choose_targets_for(inner)"
                    && !JAVA_ONLY_ROWS.contains(&text(e, "name").as_str())
            })
            .collect();
        println!("  {tag}: {} row(s) in T{turn} {phase}", list.len());
        for entry in list.iter().skip(list.len().saturating_sub(rows)) {
            println!(
                "    {tag} P{} {} -> {}",
                int(entry, "player"),
                text(entry, "name"),
                clip(&text(entry, "outcome"), 150)
            );
        }
    }
}

struct Window<'a> {
    turn: Option<i64>,
    player: Option<i64>,
    phase: Option<&'a str>,
    around: Option<&'a str>,
    before: usize,
    rows: usize,
    all: bool,
}

impl Window<'_> {
    fn keeps(&self, entry: &Value) -> bool {
        let name = text(entry, "name");
        is_callback(entry)
            && self.turn.is_none_or(|t| int(entry, "turn") == t)
            && self.player.is_none_or(|p| int(entry, "player") == p)
            && self.phase.is_none_or(|p| text(entry, "phase") == p)
            && (self.all
                || !(is_pass(entry)
                    || name == "$ACTION_SPACE"
                    || name == "choose_targets_for(inner)"
                    || JAVA_ONLY_ROWS.contains(&name.as_str())))
    }
}

fn print_window(result: &Value, window: &Window) {
    for (tag, side) in [("R", "rust_log"), ("J", "java_log")] {
        let rows: Vec<&Value> = log(result, side)
            .iter()
            .filter(|e| window.keeps(e))
            .collect();
        let start = match window.around {
            Some(needle) => match rows.iter().position(|e| {
                text(e, "name").contains(needle) || text(e, "outcome").contains(needle)
            }) {
                Some(i) => i.saturating_sub(window.before),
                None => {
                    println!("  {tag}: no row of {} mentions {needle:?}", rows.len());
                    continue;
                }
            },
            None => 0,
        };
        let end = (start + window.rows).min(rows.len());
        println!("  {tag}: rows {start}..{end} of {}", rows.len());
        for entry in &rows[start..end] {
            println!(
                "    {tag} T{} {} P{} {} -> {}",
                int(entry, "turn"),
                text(entry, "phase"),
                int(entry, "player"),
                text(entry, "name"),
                clip(&text(entry, "outcome"), 160)
            );
        }
    }
}

pub fn run_explain_cli(args: &[String]) -> i32 {
    let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: parity explain <report.json> [--seed N] [--deck2 TEXT] [--index N] [--context N]\n\
             \x20      [--turn N] [--player P] [--phase NAME] [--around TEXT] [--rows N] [--all]\n\
             <report.json> is what a run writes with `--format json -o <file>`. --turn or --around\n\
             prints both logs' callback rows instead: those of the turn, or from the first row\n\
             that mentions TEXT (a card name, `name@id`, a callback name)."
        );
        return 2;
    };
    let report: Value = match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("explain: {path}: {e}");
            return 2;
        }
    };
    let results = report
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let seed = option(args, "--seed").and_then(|s| s.parse::<i64>().ok());
    let deck2 = option(args, "--deck2");
    let index = option(args, "--index").and_then(|s| s.parse::<usize>().ok());
    let context = option(args, "--context")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(3);
    let window = Window {
        turn: option(args, "--turn").and_then(|s| s.parse().ok()),
        player: option(args, "--player").and_then(|s| s.parse().ok()),
        phase: option(args, "--phase"),
        around: option(args, "--around"),
        before: context,
        rows: option(args, "--rows")
            .and_then(|s| s.parse().ok())
            .unwrap_or(40),
        all: args.iter().any(|a| a == "--all"),
    };
    let windowed = window.turn.is_some() || window.around.is_some();

    let chosen: Vec<&Value> = match index {
        Some(i) => results.get(i).into_iter().collect(),
        None => results
            .iter()
            .filter(|r| seed.is_none_or(|s| int(r, "seed") == s))
            .filter(|r| deck2.is_none_or(|d| text(r, "deck2").contains(d)))
            .filter(|r| {
                seed.is_some()
                    || text(r, "status") != "pass"
                    || !r.get("decision").is_none_or(Value::is_null)
            })
            .collect(),
    };
    if chosen.is_empty() {
        eprintln!("explain: no game in {path} matches");
        return 1;
    }
    for result in chosen {
        println!(
            "== {} vs {} seed {}: {}",
            text(result, "deck1"),
            text(result, "deck2"),
            int(result, "seed"),
            text(result, "status")
        );
        let divergence = result.get("first_divergence").filter(|v| !v.is_null());
        if let Some(d) = divergence {
            println!(
                "state: T{} {} {} rust={} java={}",
                int(d, "turn"),
                text(d, "phase"),
                text(d, "field"),
                clip(&text(d, "rust_value"), 120),
                clip(&text(d, "java_value"), 120)
            );
        }
        let decision = result.get("decision").filter(|v| !v.is_null());
        if let Some(d) = decision {
            println!(
                "decision: T{} {}\n  rust: {}\n  java: {}\n  {}",
                int(d, "turn"),
                text(d, "phase"),
                clip(&text(d, "rust_value"), 200),
                clip(&text(d, "java_value"), 200),
                clip(&text(d, "subject"), 200)
            );
        }
        if windowed {
            print_window(result, &window);
            continue;
        }
        print_draw_diff(result, context);
        print_candidate_diff(result);
        if let Some(d) = divergence {
            print_snapshot_diff(result, int(d, "turn"));
        }
        if let Some(d) = decision {
            let (turn, phase) = (int(d, "turn"), text(d, "phase"));
            print_action_space_diff(result, turn, &phase);
            print_phase_rows(result, turn, &phase, context * 4);
        }
    }
    0
}

/// The names in a gate value printed as a list of strings; a clipped list loses its last name.
fn list_names(value: &str) -> Option<BTreeMap<String, usize>> {
    let inner = value.strip_prefix('[')?;
    let clipped = inner.ends_with('\u{2026}');
    let inner = inner.trim_end_matches('\u{2026}').trim_end_matches(']');
    let mut parts: Vec<&str> = inner
        .split("\", \"")
        .map(|p| p.trim_matches('"'))
        .filter(|p| !p.is_empty())
        .collect();
    if clipped {
        parts.pop();
    }
    let mut out = BTreeMap::new();
    for part in parts {
        *out.entry(part.to_string()).or_default() += 1;
    }
    Some(out)
}

fn one_side_names(record: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let subject = text(record, "subject");
    if !subject.is_empty() {
        out.push(subject);
    }
    if let (Some(r), Some(j)) = (
        list_names(&text(record, "rust")),
        list_names(&text(record, "java")),
    ) {
        out.extend(only_in(&r, &j));
        out.extend(only_in(&j, &r));
    }
    out
}

fn decision_callbacks(decision: &str) -> String {
    let callback = |part: &str| -> String {
        let head = part.split(" -> ").next().unwrap_or(part);
        if head.starts_with("no further decision") {
            return "(none)".to_string();
        }
        head.split_whitespace().nth(1).unwrap_or(head).to_string()
    };
    let rest = decision.split_once(": Rust ").map_or(decision, |(_, r)| r);
    let (rust, java) = rest.split_once(" / Java ").unwrap_or((rest, ""));
    format!("Rust {} / Java {}", callback(rust), callback(java))
}

fn game_label(record: &Value) -> String {
    let label = format!(
        "{} vs {} seed {}",
        text(record, "deck1"),
        text(record, "deck2"),
        int(record, "seed")
    );
    match int(record, "turn") {
        -1 => label,
        turn => format!("{label} T{turn}"),
    }
}

fn print_groups(title: &str, groups: BTreeMap<String, (Vec<String>, BTreeMap<String, usize>)>) {
    let games: usize = groups.values().map(|(g, _)| g.len()).sum();
    println!("{title}: {games} game(s) in {} group(s)", groups.len());
    let mut ordered: Vec<_> = groups.into_iter().collect();
    ordered.sort_by(|a, b| b.1 .0.len().cmp(&a.1 .0.len()).then_with(|| a.0.cmp(&b.0)));
    for (key, (games, names)) in ordered {
        println!("  {:>3}  {key}", games.len());
        if !names.is_empty() {
            let mut counted: Vec<_> = names.into_iter().collect();
            counted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let listed: Vec<String> = counted
                .iter()
                .take(12)
                .map(|(name, n)| format!("{name} {n}"))
                .collect();
            println!("       one side only: {}", listed.join(", "));
        }
        for game in games.iter().take(5) {
            println!("       {game}");
        }
        if games.len() > 5 {
            println!("       +{} more", games.len() - 5);
        }
    }
}

fn print_grouped(records: &[Value]) {
    let mut by_field: BTreeMap<String, (Vec<String>, BTreeMap<String, usize>)> = BTreeMap::new();
    let mut by_decision: BTreeMap<String, (Vec<String>, BTreeMap<String, usize>)> = BTreeMap::new();
    for record in records {
        let verdict = text(record, "verdict");
        if verdict != "PASS" {
            let field = text(record, "field");
            let key = if field.is_empty() {
                format!("{verdict} {}", clip(&text(record, "detail"), 80))
            } else {
                format!("{verdict} {field}")
            };
            let group = by_field.entry(key).or_default();
            group.0.push(game_label(record));
            for name in one_side_names(record) {
                *group.1.entry(name).or_default() += 1;
            }
        }
        let decision = text(record, "decision");
        if !decision.is_empty() {
            by_decision
                .entry(decision_callbacks(&decision))
                .or_default()
                .0
                .push(format!(
                    "{} {verdict}: {}",
                    game_label(record),
                    clip(decision.split(" (after ").next().unwrap_or(&decision), 150)
                ));
        }
    }
    print_groups("by first differing field", by_field);
    print_groups("by first differing decision", by_decision);
}

pub fn run_gate_summary_cli(args: &[String]) -> i32 {
    let Some(path) = args.get(1).filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: parity gate-summary <gate.jsonl> [--decisions] [--group]");
        return 2;
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gate-summary: {path}: {e}");
            return 2;
        }
    };
    let show_decisions = args.iter().any(|a| a == "--decisions");
    let records: Vec<Value> = raw
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let mut verdicts: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_pairing: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    let mut strict = 0usize;
    for record in &records {
        let verdict = text(record, "verdict");
        let has_decision = !record.get("decision").is_none_or(Value::is_null);
        let passed = verdict == "PASS";
        if passed && !has_decision {
            strict += 1;
        }
        *verdicts.entry(verdict).or_default() += 1;
        let pairing = by_pairing
            .entry((text(record, "deck1"), text(record, "deck2")))
            .or_default();
        pairing.0 += usize::from(passed);
        pairing.1 += 1;
    }
    println!(
        "{} game(s): {:?}; state and decisions both agree: {strict}",
        records.len(),
        verdicts
    );
    if by_pairing.len() > 1 && by_pairing.len() <= 40 {
        for ((deck1, deck2), (passed, total)) in &by_pairing {
            println!("  {deck1} vs {deck2}: {passed}/{total} PASS");
        }
    }
    if args.iter().any(|a| a == "--group") {
        print_grouped(&records);
        return i32::from(verdicts.keys().any(|v| v != "PASS"));
    }
    for record in &records {
        let verdict = text(record, "verdict");
        let decision = text(record, "decision");
        if verdict == "PASS" && (!show_decisions || decision.is_empty()) {
            continue;
        }
        println!(
            "{verdict} {} vs {} seed {} T{} {} rust={} java={}",
            text(record, "deck1"),
            text(record, "deck2"),
            int(record, "seed"),
            int(record, "turn"),
            text(record, "field"),
            clip(&text(record, "rust"), 60),
            clip(&text(record, "java"), 60)
        );
        if !decision.is_empty() {
            println!("    {}", clip(&decision, 260));
        }
    }
    i32::from(verdicts.keys().any(|v| v != "PASS"))
}
