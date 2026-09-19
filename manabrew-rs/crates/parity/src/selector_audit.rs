//! `parity selector-audit`: Java card properties that the compiled selector reads as a subtype.
//!
//! `CardProperty.cardHasProperty` falls back to a type check only after every named property;
//! the engine's selector compiler falls back to a subtype for any word it does not know, so a
//! property it has not been taught (IsSuspected) silently never matches.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_JAVA: &str = "forge/forge-game/src/main/java/forge/game/card/CardProperty.java";
const DEFAULT_SCRIPTS: &str = "forge/forge-gui/res/cardsfolder";

pub fn run_cli(args: &[String]) -> i32 {
    let mut java = PathBuf::from(DEFAULT_JAVA);
    let mut scripts = PathBuf::from(DEFAULT_SCRIPTS);
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match (arg.as_str(), iter.next()) {
            ("--java", Some(value)) => java = PathBuf::from(value),
            ("--scripts", Some(value)) => scripts = PathBuf::from(value),
            _ => {
                eprintln!("usage: parity selector-audit [--java CardProperty.java] [--scripts cardsfolder]");
                return 2;
            }
        }
    }
    let source = match std::fs::read_to_string(&java) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("selector-audit: {}: {e}", java.display());
            return 2;
        }
    };
    let names = java_property_names(&source);
    let lowered: Vec<&String> = names
        .iter()
        .filter(|name| {
            let selector =
                manabrew_engine::parsing::cached_compiled_selector(&format!("Card.{name}"));
            format!("{:?}", selector.ir).contains(&format!("Subtype({name:?})"))
        })
        .collect();
    let uses = script_uses(&scripts, &lowered);
    println!(
        "{} of {} CardProperty names compile to a subtype (scripts using each, most first):",
        lowered.len(),
        names.len()
    );
    let mut rows: Vec<(usize, &String)> = lowered
        .iter()
        .map(|name| (uses.get(name.as_str()).copied().unwrap_or(0), *name))
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    for (count, name) in rows {
        println!("{count:6}  {name}");
    }
    0
}

pub fn run_print_cli(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("usage: parity selector <selector>...");
        return 2;
    }
    for text in &args[1..] {
        let selector = manabrew_engine::parsing::cached_compiled_selector(text);
        println!("{text}\n{:#?}", selector.ir);
    }
    0
}

fn java_property_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for pattern in ["property.equals(\"", "property.equalsIgnoreCase(\""] {
        for (start, _) in source.match_indices(pattern) {
            let rest = &source[start + pattern.len()..];
            if let Some(end) = rest.find('"') {
                let name = &rest[..end];
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric())
                    && !names.iter().any(|n| n == name)
                {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

fn script_uses<'a>(dir: &Path, names: &[&'a String]) -> BTreeMap<&'a str, usize> {
    let mut uses = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for name in names {
                let used = [".", "+"].iter().any(|sep| {
                    text.match_indices(&format!("{sep}{name}")).any(|(i, m)| {
                        !text[i + m.len()..]
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphanumeric())
                    })
                });
                if used {
                    *uses.entry(name.as_str()).or_insert(0) += 1;
                }
            }
        }
    }
    uses
}
