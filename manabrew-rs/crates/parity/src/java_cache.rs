//! File-system cache for Java harness output.
//!
//! Avoids running the Java harness for matchups whose output cannot have changed.
//! Cache is keyed on:
//! - A **source hash** covering the Java sources, the card and token scripts,
//!   and the harness jar. When it changes the entire cache is wiped.
//! - Per-matchup parameters (deck1, deck2, seed, max_turns, prefer_actions,
//!   deep, variant, commanders) plus the contents of the two decks, so editing
//!   one deck only invalidates the matchups that use it.
//!
//! Individual entries are stored as JSON files so they are portable between
//! local dev, CI artefacts and Docker volumes.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::java_bridge::JavaMatchupData;
use crate::protocol::ParityLogEntry;
use crate::runner::{deck_search_dirs, RunConfig};

/// Lightweight wrapper that manages a directory of cached Java matchup outputs.
pub struct JavaCache {
    cache_dir: PathBuf,
    source_hash: String,
}

#[derive(Hash)]
struct MatchupKey<'a> {
    deck1: &'a str,
    deck1_contents: u64,
    deck2: &'a str,
    deck2_contents: u64,
    seed: u64,
    max_turns: u32,
    prefer_actions: bool,
    deep: bool,
    variant: &'a str,
    commanders: &'a [String],
}

// Minimal serde wrappers so we can store JavaMatchupData as JSON without
// requiring Serialize/Deserialize on the original struct.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedMatchup {
    log: Vec<ParityLogEntry>,
}

impl From<&JavaMatchupData> for CachedMatchup {
    fn from(d: &JavaMatchupData) -> Self {
        Self { log: d.log.clone() }
    }
}

impl From<CachedMatchup> for JavaMatchupData {
    fn from(c: CachedMatchup) -> Self {
        Self { log: c.log }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    source_hash: String,
    version: u32,
}

const MANIFEST_FILE: &str = "manifest.json";
const INCOMPLETE_HASH_PREFIX: &str = "incomplete-";
pub const CACHE_VERSION: u32 = 9;

impl JavaCache {
    /// Open (or create) a cache directory.
    ///
    /// `source_hash` is an opaque string that identifies the current Java
    /// sources, card scripts and jar. When it changes the entire cache is wiped.
    pub fn open(cache_dir: &Path, source_hash: String) -> std::io::Result<Self> {
        fs::create_dir_all(cache_dir)?;

        let manifest_path = cache_dir.join(MANIFEST_FILE);
        let needs_wipe = if manifest_path.exists() {
            match fs::read_to_string(&manifest_path) {
                Ok(s) => match serde_json::from_str::<Manifest>(&s) {
                    Ok(m) => m.source_hash != source_hash || m.version != CACHE_VERSION,
                    Err(_) => true,
                },
                Err(_) => true,
            }
        } else {
            false
        };

        if needs_wipe && source_hash.starts_with(INCOMPLETE_HASH_PREFIX) {
            eprintln!(
                "[java-cache] Not wiping {}: the source hash is missing inputs (no forge sources under the current directory, as in a git worktree); running without the cache",
                cache_dir.display()
            );
            return Err(std::io::Error::other("incomplete source hash"));
        }

        if needs_wipe {
            eprintln!(
                "[java-cache] Source hash changed — wiping cache at {}",
                cache_dir.display()
            );
            for entry in fs::read_dir(cache_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }

        let manifest = Manifest {
            source_hash: source_hash.clone(),
            version: CACHE_VERSION,
        };
        fs::write(&manifest_path, serde_json::to_string(&manifest)?)?;

        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            source_hash,
        })
    }

    /// Look up a cached matchup.  Returns `None` on miss or corruption.
    pub fn get(&self, config: &RunConfig) -> Option<JavaMatchupData> {
        let path = self.entry_path(config);
        let bytes = fs::read(&path).ok()?;
        let cached: CachedMatchup = match serde_json::from_slice(&bytes) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[java-cache] Corrupt entry {}, removing: {}",
                    path.display(),
                    e
                );
                let _ = fs::remove_file(&path);
                return None;
            }
        };
        Some(cached.into())
    }

    /// Store a matchup result.  Uses atomic write (temp + rename) to be safe
    /// under concurrent access from rayon threads.
    pub fn put(&self, config: &RunConfig, data: &JavaMatchupData) -> std::io::Result<()> {
        let path = self.entry_path(config);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let cached = CachedMatchup::from(data);
        let json = serde_json::to_vec(&cached)?;

        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &json)?;
        fs::rename(&tmp, &path)?;

        Ok(())
    }

    /// Whether the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of cached entries (for stats logging).
    pub fn len(&self) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(&self.cache_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.path().is_dir() && entry.file_name() != "." {
                for sub in fs::read_dir(entry.path()).into_iter().flatten().flatten() {
                    if sub.path().extension().map(|e| e == "json").unwrap_or(false)
                        && sub.file_name() != MANIFEST_FILE
                    {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    pub fn source_hash(&self) -> &str {
        &self.source_hash
    }

    fn entry_path(&self, config: &RunConfig) -> PathBuf {
        let decks_dirs = deck_search_dirs(config.decks_dir.as_deref());
        let key = MatchupKey {
            deck1: &config.deck1,
            deck1_contents: deck_contents_hash(&config.deck1, &decks_dirs),
            deck2: &config.deck2,
            deck2_contents: deck_contents_hash(&config.deck2, &decks_dirs),
            seed: config.seed,
            max_turns: config.max_turns,
            prefer_actions: config.prefer_actions,
            deep: config.deep,
            variant: &config.variant,
            commanders: &config.commanders,
        };
        let hash = {
            let mut h = DefaultHasher::new();
            self.source_hash.hash(&mut h);
            key.hash(&mut h);
            format!("{:016x}", h.finish())
        };
        let shard = &hash[..2];
        self.cache_dir.join(shard).join(format!("{hash}.json"))
    }
}

fn deck_contents_hash(spec: &str, decks_dirs: &[&str]) -> u64 {
    let mut hasher = DefaultHasher::new();
    if let Some(path) = spec.strip_prefix("file:") {
        fs::read(path).ok().hash(&mut hasher);
    } else if !spec.starts_with("inline:") {
        let found = decks_dirs
            .iter()
            .find_map(|dir| fs::read(Path::new(dir).join(format!("{spec}.json"))).ok());
        found.hash(&mut hasher);
    }
    hasher.finish()
}

pub fn compute_source_hash(project_root: &Path, jar_path: Option<&Path>) -> String {
    let dirs_to_hash = [
        "forge-harness/src",
        "forge/forge-game/src",
        "forge/forge-core/src",
        "forge/forge-ai/src",
        "forge/forge-gui/res/cardsfolder",
        "forge/forge-gui/res/tokenscripts",
    ];

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut missing = false;
    for dir in &dirs_to_hash {
        let full = project_root.join(dir);
        if !full.exists() {
            missing = true;
            continue;
        }
        collect_files(&full, &full, &mut files);
    }
    missing |= !project_root
        .join("forge-harness/src/main/java/forge/harness/protocol")
        .exists();
    files.sort();

    let digests: Vec<u64> = files
        .par_iter()
        .map(|(rel_path, path)| {
            let mut hasher = DefaultHasher::new();
            rel_path.hash(&mut hasher);
            fs::read(path).ok().hash(&mut hasher);
            hasher.finish()
        })
        .collect();

    let mut hasher = DefaultHasher::new();
    digests.hash(&mut hasher);
    if let Some(jar) = jar_path {
        compute_jar_hash(jar).ok().hash(&mut hasher);
    }
    let prefix = if missing { INCOMPLETE_HASH_PREFIX } else { "" };
    format!("{prefix}{:016x}", hasher.finish())
}

pub fn compute_jar_hash(jar_path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(jar_path)?;
    let mut hasher = DefaultHasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        buf[..n].hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "java" | "json" | "xml" | "properties" | "txt") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                out.push((rel, path));
            }
        }
    }
}
