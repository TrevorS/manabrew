use std::path::{Path, PathBuf};

use parity::runner::{load_data, LoadedData};
use parity::utils::decks::resolve_deck_spec;

pub const SURVEY_MATCHUPS: &str = "manabrew-rs/crates/parity/survey_matchups.tsv";
pub const DECKS_DIR: &str = "parity_decks";

#[derive(Debug, Clone)]
pub struct Deck {
    pub name: String,
    pub cards: Vec<(String, usize)>,
}

pub struct GymData {
    pub(crate) loaded: LoadedData,
    pub decks: Vec<Deck>,
}

impl GymData {
    pub fn load(deck_names: &[String], decks_dirs: &[&str]) -> Result<GymData, String> {
        let loaded = load_data(None, false)?;
        let decks = deck_names
            .iter()
            .map(|name| {
                Ok(Deck {
                    name: name.clone(),
                    cards: resolve_deck_spec(name, decks_dirs)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(GymData { loaded, decks })
    }

    pub fn survey(root: &Path) -> Result<(GymData, Vec<[usize; 2]>), String> {
        let pairs = read_matchups(&root.join(SURVEY_MATCHUPS))?;
        let mut names: Vec<String> = Vec::new();
        let mut index = |name: &str| match names.iter().position(|n| n == name) {
            Some(i) => i,
            None => {
                names.push(name.to_string());
                names.len() - 1
            }
        };
        let matchups = pairs.iter().map(|(a, b)| [index(a), index(b)]).collect();
        let dir = root.join(DECKS_DIR);
        let data = GymData::load(&names, &[&dir.to_string_lossy()])?;
        Ok((data, matchups))
    }

    pub fn deck_index(&self, name: &str) -> Option<usize> {
        self.decks.iter().position(|d| d.name == name)
    }
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn read_matchups(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.split_once('\t')
                .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
                .ok_or_else(|| format!("Bad matchup line '{l}'"))
        })
        .collect()
}
